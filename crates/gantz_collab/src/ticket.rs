//! The invite tickets. A session ticket lets others join a shared graph. A
//! vault ticket links a device to its owner's vault.

use crate::{
    runtime::PROTO_VERSION,
    session::{Access, PeerId, SessionId},
    vault::{PairingSecret, VaultId},
};
use iroh::EndpointAddr;
use iroh_tickets::{ParseError, Ticket};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Everything a peer needs to join a session. The session's identity and
/// policy, plus the sharing peer's dialable addresses.
///
/// Encodes as a `gantz…` base32 string suitable for a link or QR code. See
/// [`iroh_tickets::Ticket`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionTicket {
    /// The session to join. Seeds the gossip topic.
    pub session: SessionId,
    /// The shared graph's name.
    pub name: String,
    /// The access mode, as a hint for the joiner's UI. The serving side
    /// enforces it.
    pub access: Access,
    /// The fixed session conflict-resolution policy.
    pub resolutions: gantz_ca::merge::Resolutions,
    /// The protocol version the sharing peer speaks.
    pub proto: u32,
    /// Bootstrap addresses of the sharing peers.
    pub hosts: Vec<EndpointAddr>,
}

/// Everything a device needs to link to a vault. The vault's identity and
/// dialable address, plus the secret that pairs a new device.
///
/// Encodes as a `gantzvault…` base32 string. It carries the pairing secret,
/// so it is as sensitive as a password.
///
/// It carries no protocol version. A device keeps its ticket across
/// upgrades of itself and the vault, so the version probe decides
/// compatibility at link time. The ticket's own layout carries a version
/// on the wire, so an older build reports a ticket it cannot read.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VaultTicket {
    pub vault: VaultId,
    pub pairing: PairingSecret,
    /// The vault's address when the ticket was minted.
    pub host: EndpointAddr,
}

/// The wire form of a [`VaultTicket`]. A new layout is a new variant, so a
/// build that predates it can say that the ticket needs a newer gantz.
///
/// Variant order is part of the wire format. Append new variants at the end.
#[derive(Deserialize, Serialize)]
enum VaultTicketWire {
    V1(VaultTicket),
}

impl SessionTicket {
    /// A ticket for the current protocol version.
    pub fn new(
        session: SessionId,
        name: String,
        access: Access,
        resolutions: gantz_ca::merge::Resolutions,
        hosts: Vec<EndpointAddr>,
    ) -> Self {
        Self {
            session,
            name,
            access,
            resolutions,
            proto: PROTO_VERSION,
            hosts,
        }
    }
}

impl VaultTicket {
    /// The vault's identity.
    pub fn host_id(&self) -> PeerId {
        PeerId(*self.host.id.as_bytes())
    }
}

impl Ticket for SessionTicket {
    const KIND: &'static str = "gantz";

    fn encode_bytes(&self) -> Vec<u8> {
        crate::proto::encode(self)
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

impl Ticket for VaultTicket {
    const KIND: &'static str = "gantzvault";

    fn encode_bytes(&self) -> Vec<u8> {
        crate::proto::encode(&VaultTicketWire::V1(self.clone()))
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        match postcard::from_bytes(bytes) {
            Ok(VaultTicketWire::V1(ticket)) => Ok(ticket),
            // The leading varint is the wire variant. One this build does not
            // know comes from a newer gantz.
            Err(_) if postcard::take_from_bytes::<u32>(bytes).is_ok_and(|(v, _)| v > 0) => Err(
                ParseError::verification_failed("this ticket was made by a newer gantz"),
            ),
            Err(e) => Err(e.into()),
        }
    }
}

impl fmt::Display for SessionTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.encode_string())
    }
}

impl fmt::Display for VaultTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.encode_string())
    }
}

impl FromStr for SessionTicket {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // A vault ticket's kind extends the session kind. Reject it here
        // rather than decode its body as a session.
        if s.starts_with(VaultTicket::KIND) {
            return Err(ParseError::wrong_prefix(Self::KIND));
        }
        Self::decode_string(s)
    }
}

impl FromStr for VaultTicket {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::decode_string(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_round_trips_through_its_string_form() {
        let ticket = SessionTicket::new(
            SessionId([7; 32]),
            "jam".to_string(),
            Access::Public,
            gantz_ca::merge::Resolutions::default(),
            vec![],
        );
        let s = ticket.to_string();
        assert!(s.starts_with("gantz"));
        let parsed = SessionTicket::from_str(&s).unwrap();
        assert_eq!(parsed.session, ticket.session);
        assert_eq!(parsed.name, ticket.name);
        assert_eq!(parsed.access, ticket.access);
        assert_eq!(parsed.resolutions, ticket.resolutions);
        assert_eq!(parsed.proto, PROTO_VERSION);
    }

    #[test]
    fn vault_ticket_round_trips_and_never_parses_as_a_session() {
        let host = EndpointAddr::from(crate::Identity::generate().secret_key().public());
        let ticket = VaultTicket {
            vault: SessionId([3; 32]),
            pairing: PairingSecret::generate(),
            host,
        };
        let s = ticket.to_string();
        assert!(s.starts_with("gantzvault"));
        let parsed = VaultTicket::from_str(&s).unwrap();
        assert_eq!(parsed.vault, ticket.vault);
        assert!(parsed.pairing.matches(&ticket.pairing));
        assert_eq!(parsed.host_id(), ticket.host_id());
        assert!(SessionTicket::from_str(&s).is_err());
        let session = SessionTicket::new(
            SessionId([7; 32]),
            "jam".to_string(),
            Access::Public,
            gantz_ca::merge::Resolutions::default(),
            vec![],
        );
        assert!(VaultTicket::from_str(&session.to_string()).is_err());
    }

    // A fixed ticket's bytes are pinned, so a layout change must come as a
    // new wire variant.
    #[test]
    fn vault_ticket_v1_bytes_are_pinned() {
        // The variant, the vault id, the pairing secret, then the host.
        const V1: &str = concat!(
            "00",
            "0303030303030303030303030303030303030303030303030303030303030303",
            "0505050505050505050505050505050505050505050505050505050505050505",
            "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c00",
        );
        let pairing: PairingSecret = postcard::from_bytes(&[5; 32]).unwrap();
        let host = iroh::SecretKey::from_bytes(&[1; 32]).public();
        let ticket = VaultTicket {
            vault: SessionId([3; 32]),
            pairing,
            host: EndpointAddr::from(host),
        };
        assert_eq!(hex::encode(ticket.encode_bytes()), V1);
        let parsed = VaultTicket::decode_bytes(&hex::decode(V1).unwrap()).unwrap();
        assert_eq!(parsed.vault, ticket.vault);
    }

    #[test]
    fn a_vault_ticket_from_a_newer_layout_says_so() {
        let err = VaultTicket::decode_bytes(&[1, 0, 0]).unwrap_err();
        assert!(err.to_string().contains("newer gantz"), "{err}");
    }
}
