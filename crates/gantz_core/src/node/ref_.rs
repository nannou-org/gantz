//! A node that looks up its own implementation via content address.

use crate::datum::{self, Datum, DatumError};
use crate::{
    node::{self, Node},
    visit,
};
use gantz_nodetag::NodeTag;
use serde::de::{self, Deserialize, DeserializeOwned, Deserializer, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use std::collections::BTreeMap;
use std::fmt;

/// A node-set hook exposing the underlying [`Ref`] when a node is
/// transparently a reference to another graph (a bare [`Ref`], or a wrapper
/// such as gantz_egui's `NamedRef`).
///
/// Deliberately *not* implemented by function-value wrappers
/// ([`Fn`](crate::node::Fn)), which reference a graph without standing in for
/// it within their parent.
pub trait AsRefNode {
    /// The underlying [`Ref`], if this node is transparently a reference.
    fn as_ref_node(&self) -> Option<&Ref>;
}

/// A node that refers to another node in the environment by content address.
///
/// Reference identity is CONTENT identity: a reference to another graph
/// pins that graph's `GraphAddr`, so a referencing graph's own address
/// depends only on the content it references, never on commit history or
/// timestamps. (The address may also be a builtin node's content address -
/// resolution decides, via the environment's node lookup.)
///
/// A reference optionally carries domain-extension data in `ext`: canonical
/// [`Datum`]s keyed by a domain-prefixed string (e.g. `"plyphon.dsp-ref"`).
/// Ext data serializes and content-addresses with the node, losslessly even
/// in applications that do not know the owning domain. Conventions for ext
/// entries (see [`Ref::set_ext`]):
///
/// - Store only non-default data, so a default-configured reference keeps the
///   address it would have without the entry.
/// - One value type per key, owned by one domain.
/// - Ext data must not carry graph references: dependency collection
///   ([`Node::required_addrs`], clipboard export) cannot see inside ext, so a
///   smuggled content address would dangle.
#[derive(Clone, Debug, Eq, Hash, PartialEq, NodeTag)]
pub struct Ref {
    addr: gantz_ca::ContentAddr,
    ext: BTreeMap<String, Datum>,
}

impl Ref {
    /// Create a new [`Ref`] node that references the node at the given address.
    pub fn new(addr: gantz_ca::ContentAddr) -> Self {
        Self {
            addr,
            ext: BTreeMap::new(),
        }
    }

    /// The content address of the referenced node.
    pub fn content_addr(&self) -> gantz_ca::ContentAddr {
        self.addr
    }

    /// The same reference (including its ext data) pointing at `addr`.
    ///
    /// For repointing operations where the referenced content is equivalent
    /// (e.g. forking a reference to a new name), so domain flags still apply.
    pub fn retarget(&self, addr: gantz_ca::ContentAddr) -> Self {
        Self {
            addr,
            ext: self.ext.clone(),
        }
    }

    /// The raw extension datum stored under `key`, if any.
    pub fn ext(&self, key: &str) -> Option<&Datum> {
        self.ext.get(key)
    }

    /// Decode the extension value stored under `key` as a `T`.
    ///
    /// `None` when no entry exists or the datum does not decode as `T` (by
    /// convention a key holds one value type, so a mismatch means the entry
    /// is not the caller's).
    pub fn ext_as<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.ext
            .get(key)
            .and_then(|d| datum::from_datum(d.clone()).ok())
    }

    /// Store `value` as this reference's extension data under `key`.
    ///
    /// The value is converted to its canonical datum form once, here. See the
    /// type-level docs for the conventions ext entries follow.
    pub fn set_ext(
        &mut self,
        key: impl Into<String>,
        value: &impl Serialize,
    ) -> Result<(), DatumError> {
        let mut d = datum::to_datum(value)?;
        d.canonicalize();
        self.ext.insert(key.into(), d);
        Ok(())
    }

    /// Remove and return the extension datum stored under `key`, if any.
    pub fn remove_ext(&mut self, key: &str) -> Option<Datum> {
        self.ext.remove(key)
    }
}

/// The ext-carrying inner wire form: an explicit map of `addr` + `ext`.
struct ExtMap<'a>(&'a Ref);

impl Serialize for ExtMap<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("addr", &self.0.addr)?;
        map.serialize_entry("ext", &self.0.ext)?;
        map.end()
    }
}

// Hand-written serde keeps the ext-free wire form byte-identical to the
// previous newtype derive (a bare address, `(("hex"))` in RON, a string datum
// in the `Datum` codec). An ext-carrying reference wraps a map of
// `addr` + `ext` in the same newtype, so both forms parse through one entry
// point (RON is not self-describing at the top level; the newtype is where
// its parser branches).
impl Serialize for Ref {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.ext.is_empty() {
            s.serialize_newtype_struct("Ref", &self.addr)
        } else {
            s.serialize_newtype_struct("Ref", &ExtMap(self))
        }
    }
}

impl<'de> Deserialize<'de> for Ref {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct RefVisitor;

        impl<'de> Visitor<'de> for RefVisitor {
            type Value = Ref;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a content address or a map of `addr` + `ext`")
            }

            fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<Ref, D::Error> {
                d.deserialize_any(RefVisitor)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Ref, E> {
                let addr = v.parse::<gantz_ca::ContentAddr>().map_err(|_| {
                    de::Error::invalid_value(de::Unexpected::Str(v), &"a hex content address")
                })?;
                Ok(Ref::new(addr))
            }

            // RON's self-describing pass reports the address tuple-struct's
            // parens as a seq whose one element is the hex string.
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Ref, A::Error> {
                let hex: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                self.visit_str(&hex)
            }

            fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Ref, A::Error> {
                let mut addr = None;
                let mut ext = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "addr" => addr = Some(map.next_value::<gantz_ca::ContentAddr>()?),
                        "ext" => ext = map.next_value::<BTreeMap<String, Datum>>()?,
                        _ => {
                            map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }
                let addr = addr.ok_or_else(|| de::Error::missing_field("addr"))?;
                // Canonicalize so identity is stable regardless of how the
                // wire form ordered nested map entries.
                ext.values_mut().for_each(Datum::canonicalize);
                Ok(Ref { addr, ext })
            }
        }

        d.deserialize_newtype_struct("Ref", RefVisitor)
    }
}

impl AsRefNode for Ref {
    fn as_ref_node(&self) -> Option<&Ref> {
        Some(self)
    }
}

impl Node for Ref {
    fn n_inputs(&self, ctx: node::MetaCtx) -> usize {
        ctx.node(&self.addr).map(|n| n.n_inputs(ctx)).unwrap_or(0)
    }

    fn n_outputs(&self, ctx: node::MetaCtx) -> usize {
        ctx.node(&self.addr).map(|n| n.n_outputs(ctx)).unwrap_or(0)
    }

    fn branches(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        ctx.node(&self.addr)
            .map(|n| n.branches(ctx))
            .unwrap_or_default()
    }

    fn expr(&self, ctx: node::ExprCtx<'_, '_>) -> node::ExprResult {
        match ctx.node(&self.addr) {
            Some(n) => n.expr(ctx),
            None => Err(node::ExprError::custom(format!(
                "node not found for address {:?}",
                self.addr
            ))),
        }
    }

    fn push_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        ctx.node(&self.addr)
            .map(|n| n.push_eval(ctx))
            .unwrap_or_default()
    }

    fn pull_eval(&self, ctx: node::MetaCtx) -> Vec<node::EvalConf> {
        ctx.node(&self.addr)
            .map(|n| n.pull_eval(ctx))
            .unwrap_or_default()
    }

    fn stateful(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.addr)
            .map(|n| n.stateful(ctx))
            .unwrap_or(false)
    }

    fn register(&self, ctx: node::RegCtx<'_, '_>) {
        // Check if node exists first, then decompose context to pass to nested register.
        if ctx.node(&self.addr).is_some() {
            let (get_node, path, vm) = ctx.into_parts();
            // Safe to unwrap since we checked above.
            let n = (get_node)(&self.addr).unwrap();
            n.register(node::RegCtx::new(get_node, path, vm));
        }
    }

    fn inlet(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.addr).map(|n| n.inlet(ctx)).unwrap_or(false)
    }

    fn outlet(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.addr).map(|n| n.outlet(ctx)).unwrap_or(false)
    }

    fn delay(&self, ctx: node::MetaCtx) -> bool {
        ctx.node(&self.addr).map(|n| n.delay(ctx)).unwrap_or(false)
    }

    fn required_addrs(&self) -> Vec<gantz_ca::ContentAddr> {
        vec![self.addr]
    }

    fn visit(&self, ctx: visit::Ctx<'_, '_>, visitor: &mut dyn node::Visitor) {
        if let Some(n) = ctx.node(&self.addr) {
            n.visit(ctx, visitor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    fn test_ref() -> Ref {
        Ref::new(gantz_ca::ContentAddr::from([0u8; 32]))
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestExt {
        zeta: bool,
        alpha: u32,
    }

    /// Both wire shapes round-trip through the Datum codec: ext-free stays
    /// the bare address string, ext-carrying takes the map form. The stored
    /// canonical form survives (struct field order does not leak).
    #[test]
    fn ref_roundtrips_through_datum_codec() {
        let plain = test_ref();
        let d = datum::to_datum(&plain).unwrap();
        assert!(
            matches!(d, Datum::Str(_)),
            "ext-free Ref must stay a bare address, got {d:?}"
        );
        assert_eq!(datum::from_datum::<Ref>(d).unwrap(), plain);

        let mut extended = test_ref();
        extended
            .set_ext(
                "test.ext",
                &TestExt {
                    zeta: true,
                    alpha: 3,
                },
            )
            .unwrap();
        let d = datum::to_datum(&extended).unwrap();
        assert!(
            matches!(d, Datum::Map(_)),
            "ext-carrying Ref must take the map form, got {d:?}"
        );
        let rt: Ref = datum::from_datum(d).unwrap();
        assert_eq!(rt, extended);
        // The stored datum is canonical despite TestExt's declaration order.
        assert_eq!(
            extended.ext("test.ext"),
            Some(&Datum::Map(vec![
                ("alpha".to_string(), Datum::U64(3)),
                ("zeta".to_string(), Datum::Bool(true)),
            ])),
        );
        assert_eq!(
            extended.ext_as::<TestExt>("test.ext"),
            Some(TestExt {
                zeta: true,
                alpha: 3
            })
        );
    }

    /// serde_json coverage for the second self-describing format: both
    /// shapes round-trip exactly.
    #[test]
    fn ref_roundtrips_through_json() {
        let mut extended = test_ref();
        extended
            .set_ext(
                "test.ext",
                &TestExt {
                    zeta: false,
                    alpha: 1,
                },
            )
            .unwrap();
        for r in [test_ref(), extended] {
            let json = serde_json::to_string(&r).unwrap();
            let rt: Ref = serde_json::from_str(&json).unwrap();
            assert_eq!(rt, r);
        }
    }
}
