//! The session invite ticket. The string a user shares to let others join.

use crate::{
    runtime::PROTO_VERSION,
    session::{Access, SessionId},
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

impl Ticket for SessionTicket {
    const KIND: &'static str = "gantz";

    fn encode_bytes(&self) -> Vec<u8> {
        crate::proto::encode(self)
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

impl fmt::Display for SessionTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.encode_string())
    }
}

impl FromStr for SessionTicket {
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
    fn garbage_tickets_are_rejected() {
        assert!(SessionTicket::from_str("gantznotaticket").is_err());
        assert!(SessionTicket::from_str("blob123").is_err());
    }
}
