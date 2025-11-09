//! The content-addressing implementation for `gantz` graphs.

#[doc(inline)]
pub use hash::CaHash;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, hash::Hash, ops, str};

pub mod hash;

/// The content address of a graph.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ContentAddr(
    #[serde(serialize_with = "serialize", deserialize_with = "deserialize")] pub [u8; 32],
);

/// Provides a `Display` implementation that formats the CA into shorthand form.
#[derive(Clone, Copy, Debug)]
pub struct ContentAddrShort<'a>(&'a ContentAddr);

/// The [`blake3`] hasher used for gantz' content addressing.
pub type Hasher = blake3::Hasher;

impl ContentAddr {
    /// Provides a `Display` implementation that formats the CA into shorthand form.
    pub fn display_short(&self) -> ContentAddrShort<'_> {
        ContentAddrShort(self)
    }
}

impl AsRef<[u8; 32]> for ContentAddr {
    fn as_ref(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ContentAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl<'a> fmt::Display for ContentAddrShort<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = hex::encode(self.0.0);
        s.truncate(8);
        write!(f, "{s}")
    }
}

impl From<ContentAddr> for [u8; 32] {
    fn from(ContentAddr(bytes): ContentAddr) -> Self {
        bytes
    }
}

impl From<[u8; 32]> for ContentAddr {
    fn from(bytes: [u8; 32]) -> Self {
        ContentAddr(bytes)
    }
}

impl ops::Deref for ContentAddr {
    type Target = [u8; 32];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl str::FromStr for ContentAddr {
    type Err = hex::FromHexError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let vec = hex::decode(s)?;
        let bytes: [u8; 32] = vec
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)?;
        Ok(ContentAddr(bytes))
    }
}

/// Hash some type implementing [`Hash`].
pub fn content_addr<T: CaHash>(t: &T) -> ContentAddr {
    let mut hasher = Hasher::new();
    t.hash(&mut hasher);
    ContentAddr(hasher.finalize().into())
}

/// Serialize a fixed-size hash value.
fn serialize<S>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if s.is_human_readable() {
        let string = hex::encode(bytes);
        string.serialize(s)
    } else {
        bytes[..].serialize(s)
    }
}

/// Deserialize a `ContentAddress`.
fn deserialize<'de, D>(d: D) -> Result<[u8; 32], D::Error>
where
    D: Deserializer<'de>,
{
    let bytes: Vec<u8> = if d.is_human_readable() {
        let string = String::deserialize(d)?;
        hex::decode(string).map_err(serde::de::Error::custom)?
    } else {
        Vec::deserialize(d)?
    };
    let len = bytes.len();
    bytes.try_into().map_err(|_err| {
        let msg = format!("failed to convert `Vec<u8>` with length {len} to `[u8; 32]`");
        serde::de::Error::custom(msg)
    })
}
