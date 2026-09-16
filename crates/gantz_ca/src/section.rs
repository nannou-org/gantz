//! The open extension surface of the registry. Mutable metadata sections
//! and content-addressed blob stores.
//!
//! A domain extends the registry by declaring a [`SectionDecl`] or a
//! [`BlobDecl`] in its own crate. A section holds keyed, mutable metadata
//! such as descriptions or views. A blob store holds content-addressed
//! opaque bytes such as audio samples. The declaration is compile-time
//! only. Nothing registers at runtime, and the registry core never learns
//! domain types.
//!
//! Section semantics travel as data. The merge policy and liveness rule are
//! stamped into the section when it is first written. An application or
//! peer without the owning domain compiled in still merges, prunes, exports
//! and round-trips the section correctly without interpreting its values.
//!
//! Blob addresses are the blake3 hash of the raw bytes and nothing else.
//! There is no kind tag and no framing. See [`blob_addr`]. This keeps them
//! bit-compatible with iroh-blobs content addressing, which preserves
//! verified-streaming transfer as a future option. Blob metadata belongs
//! beside the blob in a metadata section, never in the hashed bytes.

use crate::{CommitAddr, ContentAddr, GraphAddr, datum::Datum};
use crate::{Name, datum};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeMap, sync::Arc};

/// Identifies a section, by convention `<domain>.<kind>`. For example,
/// `egui.view` or `dsp.buffer`. The `heads` section is core.
pub type SectionId = String;

/// Cheaply clonable immutable blob bytes.
pub type Bytes = Arc<[u8]>;

/// A typed declaration of a metadata section, implemented by the owning
/// domain crate. See the module docs.
pub trait SectionDecl {
    /// The section's identifier, by convention `<domain>.<kind>`.
    const ID: &'static str;
    /// The merge policy stamped into the section on first write.
    const POLICY: MergePolicy;
    /// The liveness rule stamped into the section on first write.
    const LIVENESS: Liveness;
    /// The key type. It converts through [`Key`].
    type Key: Into<Key> + TryFromKey;
    /// The value type. It encodes through [`Datum`] by default.
    type Value: Serialize + serde::de::DeserializeOwned;

    /// Encode a typed value as a section [`Value`]. Defaults to an inline
    /// datum. Override to use a typed [`Value`] form. For example, the
    /// `heads` section encodes values as [`Value::Commit`].
    fn encode(value: &Self::Value) -> Result<Value, datum::DatumError> {
        value_to_datum(value)
    }

    /// Decode a typed value from a section [`Value`]. `None` on shape
    /// mismatch. Must invert [`encode`](Self::encode).
    fn decode(value: &Value) -> Option<Self::Value> {
        value_from_datum(value)
    }
}

/// A typed declaration of a blob store section, implemented by the owning
/// domain crate.
pub trait BlobDecl {
    /// The section's identifier, by convention `<domain>.<kind>`.
    const ID: &'static str;
    /// The liveness rule stamped into the store on first write.
    const LIVENESS: BlobLiveness;
}

/// Fallible conversion out of the erased [`Key`], used by typed accessors.
pub trait TryFromKey: Sized {
    fn try_from_key(key: &Key) -> Option<Self>;
}

/// What a section's entries are keyed by.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Key {
    /// Keyed by a head name. Such metadata survives edits to the named line
    /// of history. For example, descriptions.
    Name(Name),
    /// Keyed by a commit. Such metadata is about a specific history point.
    /// For example, scene views.
    Commit(CommitAddr),
    /// Keyed by a graph. Such metadata is about specific content.
    Graph(GraphAddr),
    /// Keyed by an arbitrary content address. For example, blob metadata.
    Addr(ContentAddr),
}

/// A section entry's value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Value {
    /// An inline, self-describing canonical value.
    Datum(Datum),
    /// An indirection into a blob store section, for large values.
    Blob(SectionId, ContentAddr),
    /// A typed pointer into history. This is the `heads` section's value
    /// form.
    Commit(CommitAddr),
}

/// How a section's entries merge when an incoming registry is merged in.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MergePolicy {
    /// A present local entry wins. Used for descriptions, demos and views.
    KeepExisting,
    /// A differing incoming entry replaces the local one and is reported in
    /// the merge result. Used for the `heads` section.
    Replace,
}

/// When a section entry is live. Dead entries are dropped by prune and
/// omitted from exports.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Liveness {
    /// Entries are garbage-collection roots. Their commit values and
    /// everything reachable from them are kept alive. Used for the `heads`
    /// section.
    Root,
    /// Live while the `heads` section contains the entry's `Key::Name`.
    WithName,
    /// Live while the entry's `Key::Commit` commit exists.
    WithCommit,
    /// Live while the entry's `Key::Graph` graph exists.
    WithGraph,
    /// Never automatically dropped.
    Pinned,
}

/// When a blob store entry is live.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BlobLiveness {
    /// Live while referenced by live content, such as a graph node reference.
    ContentReferenced,
    /// Live while referenced by a live section entry via [`Value::Blob`].
    SectionReferenced,
    /// Never automatically dropped.
    Pinned,
}

/// A mutable, keyed metadata section. All registry mutability outside the
/// content columns lives in sections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Section {
    /// The merge policy, stamped on first write. Stored metadata wins over
    /// compile-time constants so mixed application versions agree.
    pub policy: MergePolicy,
    /// The liveness rule, stamped on first write.
    pub liveness: Liveness,
    /// The entries. A `BTreeMap` for deterministic serialization.
    pub entries: BTreeMap<Key, Value>,
}

/// A content-addressed store of opaque canonical bytes.
///
/// Every key equals [`blob_addr`] of its bytes. [`insert`] computes the key
/// and [`insert_at`] verifies a claimed key.
///
/// [`insert`]: Self::insert
/// [`insert_at`]: Self::insert_at
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlobStore {
    /// The liveness rule, stamped on first write.
    pub liveness: BlobLiveness,
    /// The entries. A `BTreeMap` for deterministic serialization.
    #[serde(
        serialize_with = "serialize_blob_entries",
        deserialize_with = "deserialize_blob_entries"
    )]
    pub entries: BTreeMap<ContentAddr, Bytes>,
}

/// A blob claimed under an address it does not hash to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobVerifyError {
    pub claimed: ContentAddr,
    pub actual: ContentAddr,
}

impl Section {
    /// An empty section with the given stamped semantics.
    pub fn new(policy: MergePolicy, liveness: Liveness) -> Self {
        Self {
            policy,
            liveness,
            entries: BTreeMap::new(),
        }
    }
}

impl BlobStore {
    /// An empty store with the given stamped liveness.
    pub fn new(liveness: BlobLiveness) -> Self {
        Self {
            liveness,
            entries: BTreeMap::new(),
        }
    }

    /// Insert bytes, computing their address. Idempotent, since an existing
    /// entry for the address is identical by construction.
    pub fn insert(&mut self, bytes: impl Into<Bytes>) -> ContentAddr {
        let bytes = bytes.into();
        let addr = blob_addr(&bytes);
        self.entries.entry(addr).or_insert(bytes);
        addr
    }

    /// Insert bytes under a claimed address, verifying the claim.
    pub fn insert_at(
        &mut self,
        claimed: ContentAddr,
        bytes: impl Into<Bytes>,
    ) -> Result<(), BlobVerifyError> {
        let bytes = bytes.into();
        let actual = blob_addr(&bytes);
        if actual != claimed {
            return Err(BlobVerifyError { claimed, actual });
        }
        self.entries.entry(claimed).or_insert(bytes);
        Ok(())
    }

    /// The bytes stored at the given address, if any.
    pub fn get(&self, addr: &ContentAddr) -> Option<&Bytes> {
        self.entries.get(addr)
    }
}

impl TryFromKey for Name {
    fn try_from_key(key: &Key) -> Option<Self> {
        match key {
            Key::Name(name) => Some(name.clone()),
            _ => None,
        }
    }
}

impl TryFromKey for CommitAddr {
    fn try_from_key(key: &Key) -> Option<Self> {
        match key {
            Key::Commit(ca) => Some(*ca),
            _ => None,
        }
    }
}

impl TryFromKey for GraphAddr {
    fn try_from_key(key: &Key) -> Option<Self> {
        match key {
            Key::Graph(ga) => Some(*ga),
            _ => None,
        }
    }
}

impl TryFromKey for ContentAddr {
    fn try_from_key(key: &Key) -> Option<Self> {
        match key {
            Key::Addr(addr) => Some(*addr),
            _ => None,
        }
    }
}

impl From<Name> for Key {
    fn from(name: Name) -> Self {
        Key::Name(name)
    }
}

impl From<CommitAddr> for Key {
    fn from(ca: CommitAddr) -> Self {
        Key::Commit(ca)
    }
}

impl From<GraphAddr> for Key {
    fn from(ga: GraphAddr) -> Self {
        Key::Graph(ga)
    }
}

impl std::fmt::Display for BlobVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "blob claimed as {} hashes to {}",
            self.claimed, self.actual
        )
    }
}

impl std::error::Error for BlobVerifyError {}

/// The content address of a blob. The blake3 hash of the raw bytes and
/// nothing else. No kind tag, no length prefix, no framing. This is the
/// iroh-blobs-compatible addressing rule. See the module docs.
pub fn blob_addr(bytes: &[u8]) -> ContentAddr {
    ContentAddr::from(*blake3::hash(bytes).as_bytes())
}

/// Encode a typed section value as an inline datum [`Value`].
pub fn value_to_datum<T: Serialize>(value: &T) -> Result<Value, datum::DatumError> {
    datum::to_datum(value).map(Value::Datum)
}

/// Decode a typed section value from a [`Value`], if it is an inline datum
/// of the expected shape.
pub fn value_from_datum<T: serde::de::DeserializeOwned>(value: &Value) -> Option<T> {
    match value {
        Value::Datum(datum) => datum::from_datum(datum.clone()).ok(),
        _ => None,
    }
}

/// Serialize blob entries with hex addresses and hex bytes when
/// human-readable, raw bytes otherwise. Entries are ordered by address.
fn serialize_blob_entries<S: Serializer>(
    entries: &BTreeMap<ContentAddr, Bytes>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    /// Serializes a byte slice via `serialize_bytes` rather than serde's
    /// default seq-of-u8 for slices.
    struct AsBytes<'a>(&'a [u8]);
    impl Serialize for AsBytes<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(self.0)
        }
    }
    let human_readable = serializer.is_human_readable();
    let mut map = serializer.serialize_map(Some(entries.len()))?;
    for (addr, bytes) in entries {
        if human_readable {
            map.serialize_entry(addr, &hex::encode(&bytes[..]))?;
        } else {
            map.serialize_entry(addr, &AsBytes(bytes))?;
        }
    }
    map.end()
}

/// Deserialize blob entries. See [`serialize_blob_entries`].
fn deserialize_blob_entries<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<ContentAddr, Bytes>, D::Error> {
    use serde::de::Error;
    if deserializer.is_human_readable() {
        let hex_entries = BTreeMap::<ContentAddr, String>::deserialize(deserializer)?;
        hex_entries
            .into_iter()
            .map(|(addr, hex_str)| {
                let bytes = hex::decode(&hex_str).map_err(D::Error::custom)?;
                Ok((addr, Bytes::from(bytes)))
            })
            .collect()
    } else {
        let raw = BTreeMap::<ContentAddr, Vec<u8>>::deserialize(deserializer)?;
        Ok(raw
            .into_iter()
            .map(|(addr, bytes)| (addr, Bytes::from(bytes)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_addr_is_raw_blake3() {
        // The iroh interop guard. The address must be the plain blake3 hash
        // of the raw bytes, with nothing folded into the preimage.
        let bytes = b"hello gantz";
        let addr = blob_addr(bytes);
        assert_eq!(addr.as_ref(), blake3::hash(bytes).as_bytes());
    }

    #[test]
    fn blob_store_verifies_claims() {
        let mut store = BlobStore::new(BlobLiveness::ContentReferenced);
        let addr = store.insert(&b"content"[..]);
        assert!(store.insert_at(addr, &b"content"[..]).is_ok());
        let err = store.insert_at(addr, &b"tampered"[..]).unwrap_err();
        assert_eq!(err.claimed, addr);
        assert_ne!(err.actual, addr);
        assert_eq!(store.get(&addr).map(|b| &b[..]), Some(&b"content"[..]));
    }

    #[test]
    fn blob_store_serde_round_trips() {
        let mut store = BlobStore::new(BlobLiveness::SectionReferenced);
        store.insert(&b"alpha"[..]);
        store.insert(&b"beta"[..]);
        let ron = ron::to_string(&store).unwrap();
        let back: BlobStore = ron::de::from_str(&ron).unwrap();
        assert_eq!(back, store);
    }

    #[test]
    fn section_serde_round_trips_with_semantics() {
        let mut section = Section::new(MergePolicy::KeepExisting, Liveness::WithName);
        section.entries.insert(
            Key::Name("synth".parse().unwrap()),
            value_to_datum(&"a bass synth".to_string()).unwrap(),
        );
        let ron = ron::to_string(&section).unwrap();
        let back: Section = ron::de::from_str(&ron).unwrap();
        assert_eq!(back, section);
        let decoded: String =
            value_from_datum(&back.entries[&Key::Name("synth".parse().unwrap())]).unwrap();
        assert_eq!(decoded, "a bass synth");
    }

    #[test]
    fn typed_value_mismatch_decodes_to_none() {
        let value = value_to_datum(&42u32).unwrap();
        let decoded: Option<Vec<String>> = value_from_datum(&value);
        assert!(decoded.is_none());
    }
}
