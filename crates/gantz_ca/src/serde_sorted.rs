//! Sorted serialization helpers for `HashMap` fields.
//!
//! These functions serialize `HashMap`s with keys in sorted order, producing
//! deterministic output without sacrificing O(1) lookup at runtime. A transient
//! `BTreeMap<&K, &V>` is built from borrowed references - no cloning required.

use serde::{Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// Serialize a `HashMap` with keys in sorted order.
pub fn serialize_map<K, V, S>(map: &HashMap<K, V>, s: S) -> Result<S::Ok, S::Error>
where
    K: Serialize + Ord + Hash,
    V: Serialize,
    S: Serializer,
{
    let sorted: BTreeMap<&K, &V> = map.iter().collect();
    sorted.serialize(s)
}

/// Serialize a `HashMap` of `HashMap`s with keys sorted at both levels.
pub fn serialize_map_of_maps<K1, K2, V, S>(
    map: &HashMap<K1, HashMap<K2, V>>,
    s: S,
) -> Result<S::Ok, S::Error>
where
    K1: Serialize + Ord + Hash,
    K2: Serialize + Ord + Hash,
    V: Serialize,
    S: Serializer,
{
    let sorted: BTreeMap<&K1, BTreeMap<&K2, &V>> =
        map.iter().map(|(k, v)| (k, v.iter().collect())).collect();
    sorted.serialize(s)
}
