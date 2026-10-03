//! The version probe. Every runtime answers it on [`VERSION_ALPN`], so a
//! peer learns whether the two can sync, and which side needs an update,
//! before it speaks the sync protocol.
//!
//! The probe must work between any two gantz builds, so its ALPN and its
//! exchange are frozen. Each side writes one [`VersionInfo`] as RON with a
//! default for every field, then reads the other side's. A field added later
//! reads as its default on an older build and is ignored by it.
//!
//! Compatibility is decided on the sync protocol range alone. The app
//! version is for display, since development builds share one version.

use crate::runtime::PROTO_VERSION;
use crate::session::PeerId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The ALPN of the version probe. Frozen, unlike the versioned
/// [`SYNC_ALPN`](crate::SYNC_ALPN).
pub const VERSION_ALPN: &[u8] = b"gantz/version";

/// The oldest sync protocol version this build speaks.
pub const PROTO_MIN: u32 = PROTO_VERSION;

/// The newest sync protocol version this build speaks.
pub const PROTO_MAX: u32 = PROTO_VERSION;

/// The most a probe reads from its peer. [`VersionInfo`] is tiny.
const PROBE_LIMIT: usize = 4 * 1024;

/// How long a probe waits for its peer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// What a peer speaks, and which build it is.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct VersionInfo {
    /// The oldest sync protocol version the peer speaks.
    pub proto_min: u32,
    /// The newest sync protocol version the peer speaks.
    pub proto_max: u32,
    /// The peer's app and version, such as `gantz 0.4.0`, for display.
    pub app: String,
}

/// Which of two peers that share no protocol version must update.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outdated {
    /// This peer, since the other only speaks newer protocol versions.
    Us,
    /// The other peer, since it only speaks older protocol versions.
    Them,
}

/// Answers the version probe with this build's [`VersionInfo`]. It emits
/// each probing peer's info, so a vault can report devices that cannot
/// link.
#[derive(Clone, Debug)]
pub(crate) struct VersionServer {
    pub info: VersionInfo,
    pub events: async_channel::Sender<crate::Event>,
}

impl VersionInfo {
    /// This build's info.
    pub fn this(app: &str) -> Self {
        Self {
            proto_min: PROTO_MIN,
            proto_max: PROTO_MAX,
            app: app.to_string(),
        }
    }

    /// Encode as the probe's RON.
    fn encode(&self) -> Vec<u8> {
        ron::to_string(self).unwrap_or_default().into_bytes()
    }

    /// Decode the probe's RON.
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        ron::de::from_bytes(bytes).map_err(|e| format!("unreadable version info: {e}"))
    }
}

/// The newest protocol version that `ours` and `theirs` both speak, or
/// which of them must update.
pub(crate) fn compat(ours: &VersionInfo, theirs: &VersionInfo) -> Result<u32, Outdated> {
    let newest_shared = ours.proto_max.min(theirs.proto_max);
    if newest_shared >= ours.proto_min.max(theirs.proto_min) {
        Ok(newest_shared)
    } else if theirs.proto_max < ours.proto_min {
        Err(Outdated::Them)
    } else {
        Err(Outdated::Us)
    }
}

/// Ask the peer at `addr` what it speaks, telling it what this build
/// speaks. Uses its own connection, since connections are cached per peer
/// and a probe connection cannot carry sync requests.
pub(crate) async fn probe(
    endpoint: &Endpoint,
    addr: EndpointAddr,
    ours: &VersionInfo,
) -> Result<VersionInfo, String> {
    let exchange = async {
        let conn = endpoint
            .connect(addr, VERSION_ALPN)
            .await
            .map_err(|e| format!("connect failed: {e}"))?;
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| format!("open failed: {e}"))?;
        send.write_all(&ours.encode())
            .await
            .map_err(|e| format!("send failed: {e}"))?;
        let _ = send.finish();
        let bytes = recv
            .read_to_end(PROBE_LIMIT)
            .await
            .map_err(|e| format!("receive failed: {e}"))?;
        conn.close(0u32.into(), b"done");
        VersionInfo::decode(&bytes)
    };
    n0_future::time::timeout(PROBE_TIMEOUT, exchange)
        .await
        .unwrap_or_else(|_| Err("the version probe timed out".to_string()))
}

impl ProtocolHandler for VersionServer {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let peer = PeerId(*conn.remote_id().as_bytes());
        let Ok((mut send, mut recv)) = conn.accept_bi().await else {
            return Ok(());
        };
        // A probe from a build that sends nothing still gets an answer.
        let theirs = recv
            .read_to_end(PROBE_LIMIT)
            .await
            .ok()
            .and_then(|bytes| VersionInfo::decode(&bytes).ok())
            .unwrap_or_default();
        let outdated = compat(&self.info, &theirs).err();
        let probed = crate::Event::Probed {
            peer,
            info: theirs,
            outdated,
        };
        let _ = self.events.try_send(probed);
        if send.write_all(&self.info.encode()).await.is_ok() {
            let _ = send.finish();
        }
        // Wait for the prober to read the answer and close.
        conn.closed().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(proto_min: u32, proto_max: u32) -> VersionInfo {
        VersionInfo {
            proto_min,
            proto_max,
            app: String::new(),
        }
    }

    #[test]
    fn compat_picks_the_newest_shared_protocol_or_who_must_update() {
        let cases = [
            (info(2, 2), info(2, 2), Ok(2)),
            (info(2, 3), info(2, 2), Ok(2)),
            (info(2, 3), info(3, 4), Ok(3)),
            (info(3, 3), info(2, 2), Err(Outdated::Them)),
            (info(2, 2), info(3, 3), Err(Outdated::Us)),
        ];
        for (ours, theirs, expected) in cases {
            assert_eq!(compat(&ours, &theirs), expected, "{ours:?} vs {theirs:?}");
        }
    }

    // Builds on either side of a field change still read each other.
    #[test]
    fn version_info_reads_older_and_newer_shapes() {
        let older = VersionInfo::decode(b"(proto_min:2,proto_max:2)").unwrap();
        assert_eq!(older, info(2, 2));
        let newer = b"(proto_min:2,proto_max:5,app:\"gantz 9.0.0\",features:[\"x\"])";
        let newer = VersionInfo::decode(newer).unwrap();
        assert_eq!(newer.proto_max, 5);
        assert_eq!(newer.app, "gantz 9.0.0");
        let this = VersionInfo::this("gantz 0.4.0");
        assert_eq!(VersionInfo::decode(&this.encode()).unwrap(), this);
    }
}
