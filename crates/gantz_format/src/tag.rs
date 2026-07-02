//! Deterministic serde dispatch for `Box<dyn Node>`-style node sets.
//!
//! Each node type declares its wire tag - the `"type"` entry of its
//! serialized map - at its own definition site via [`NodeTag`], and an
//! application composes its node set with [`impl_node_set_serde!`], which
//! generates `Serialize`/`Deserialize` for the erased `Box<dyn Trait>` as a
//! compiled match over the listed types. There is no runtime registry: unlike
//! `typetag`'s `inventory`-based registration (whose life-before-main
//! constructors the WASM linker can silently discard - see gantz#181),
//! nothing here can be dropped at link time.

/// A node type's wire tag: the value of the `"type"` entry in its serialized
/// map form, e.g. `(node "Expr" ...)` in `.gantz` text.
///
/// Declared alongside the node type itself (like its `#[cahash(...)]`
/// discriminator and its [`Sugar`](crate::Sugar) keyword) so that every
/// application composing the node set agrees on the same wire format.
/// `gantz_format` provides the impls for `gantz_core`'s nodes; downstream
/// crates provide their own.
///
/// Tags are part of the wire format: changing one breaks the loading of
/// existing `.gantz` exports and persisted registries that contain the node.
pub trait NodeTag {
    /// The `"type"` tag identifying this node type on the wire.
    const TAG: &'static str;
}

impl NodeTag for gantz_core::node::Apply {
    const TAG: &'static str = "Apply";
}

impl NodeTag for gantz_core::node::Branch {
    const TAG: &'static str = "Branch";
}

impl NodeTag for gantz_core::node::Delay {
    const TAG: &'static str = "Delay";
}

impl NodeTag for gantz_core::node::Expr {
    const TAG: &'static str = "Expr";
}

impl NodeTag for gantz_core::node::Identity {
    const TAG: &'static str = "Identity";
}

impl NodeTag for gantz_core::node::graph::Inlet {
    const TAG: &'static str = "Inlet";
}

impl NodeTag for gantz_core::node::graph::Outlet {
    const TAG: &'static str = "Outlet";
}

/// The wire tag for [`Fn<Self>`](gantz_core::node::Fn), for node types that
/// appear fn-wrapped in a node set.
///
/// `Fn<N>` is foreign to `N`'s crate, so the orphan rule forbids implementing
/// [`NodeTag`] for it there directly; this lets the wrapped type declare the
/// wrapper's tag at its own definition site instead, and the blanket impl
/// below lifts it.
pub trait FnNodeTag {
    /// The `"type"` tag identifying `Fn<Self>` on the wire.
    const FN_TAG: &'static str;
}

impl<N: FnNodeTag> NodeTag for gantz_core::node::Fn<N> {
    const TAG: &'static str = N::FN_TAG;
}

/// The tag-first map wrapper the generated `Serialize` uses: `flatten` forces
/// `serialize_map`, reproducing the exact wire shape `typetag` produced (a
/// unit-struct node flattens to nothing, leaving a tag-only map).
///
/// Public for the macro expansion only; not part of the crate's API.
#[doc(hidden)]
#[derive(serde::Serialize)]
pub struct TaggedNode<'a, T: serde::Serialize> {
    pub r#type: &'static str,
    #[serde(flatten)]
    pub node: &'a T,
}

/// Implement `Serialize`/`Deserialize` for a node set's `Box<dyn Trait>` by
/// dispatching on each listed type's [`NodeTag`].
///
/// The serialized form is a map carrying the node's `TAG` under a `"type"`
/// entry alongside the node's own fields (`typetag`-compatible, but with no
/// runtime registry - dispatch compiles to a plain match, so it cannot be
/// broken by link-time dead-code elimination on WASM; see gantz#181).
///
/// The trait must have [`std::any::Any`] as a (transitive) supertrait, and
/// the calling crate must depend on `serde`. Adding a node type to an
/// application is: implement [`NodeTag`] beside the type, then add one line
/// here - a round-trip gate test over the full node set is the recommended
/// guard against forgetting the latter.
///
/// ```
/// trait Node: std::any::Any {}
///
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct Gain {
///     db: f64,
/// }
///
/// impl gantz_format::NodeTag for Gain {
///     const TAG: &'static str = "Gain";
/// }
///
/// impl Node for Gain {}
///
/// gantz_format::impl_node_set_serde! {
///     dyn Node {
///         Gain,
///     }
/// }
///
/// let node: Box<dyn Node> = Box::new(Gain { db: -6.0 });
/// let datum = gantz_format::to_datum(&node).unwrap();
/// let back: Box<dyn Node> = gantz_format::from_datum(datum.clone()).unwrap();
/// assert_eq!(gantz_format::to_datum(&back).unwrap(), datum);
/// ```
#[macro_export]
macro_rules! impl_node_set_serde {
    (dyn $trait_:path { $($ty:ty),+ $(,)? }) => {
        impl ::serde::Serialize for ::std::boxed::Box<dyn $trait_> {
            fn serialize<S>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                let any: &dyn ::std::any::Any = &**self;
                $(
                    if let ::std::option::Option::Some(node) = any.downcast_ref::<$ty>() {
                        let tagged = $crate::TaggedNode {
                            r#type: <$ty as $crate::NodeTag>::TAG,
                            node,
                        };
                        return ::serde::Serialize::serialize(&tagged, serializer);
                    }
                )+
                // A nested box (`Box<dyn Trait>` typically implements the
                // trait itself) delegates to the inner node's tag.
                if let ::std::option::Option::Some(nested) =
                    any.downcast_ref::<::std::boxed::Box<dyn $trait_>>()
                {
                    return ::serde::Serialize::serialize(nested, serializer);
                }
                ::std::result::Result::Err(::serde::ser::Error::custom(::std::concat!(
                    "cannot serialize `Box<dyn ",
                    ::std::stringify!($trait_),
                    ">`: concrete type not listed in `impl_node_set_serde!`",
                )))
            }
        }

        impl<'de> ::serde::Deserialize<'de> for ::std::boxed::Box<dyn $trait_> {
            fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
            where
                D: ::serde::Deserializer<'de>,
            {
                // Buffer the self-describing input, then let the `type` tag
                // select which concrete type's `Deserialize` the remaining
                // fields drive.
                let datum = <$crate::Datum as ::serde::Deserialize>::deserialize(deserializer)?;
                let (tag, fields) = datum.split_tagged().map_err(::serde::de::Error::custom)?;
                $(
                    if tag == <$ty as $crate::NodeTag>::TAG {
                        return $crate::from_datum::<$ty>(fields)
                            .map(|node| {
                                ::std::boxed::Box::new(node) as ::std::boxed::Box<dyn $trait_>
                            })
                            .map_err(::serde::de::Error::custom);
                    }
                )+
                const EXPECTED: &[&str] = &[$(<$ty as $crate::NodeTag>::TAG),+];
                ::std::result::Result::Err(::serde::de::Error::unknown_variant(&tag, EXPECTED))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Datum, from_datum, node_datum, to_datum};

    /// The dispatch handles all three node struct shapes: unit, fields and
    /// newtype (`Fn<N>` delegates to the wrapped node's fields).
    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Unit;

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Fields {
        a: i64,
        b: String,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Newtype(Fields);

    trait TestNode: std::any::Any {}

    impl TestNode for Unit {}
    impl TestNode for Fields {}
    impl TestNode for Newtype {}
    impl TestNode for Box<dyn TestNode> {}

    impl NodeTag for Unit {
        const TAG: &'static str = "Unit";
    }

    impl NodeTag for Fields {
        const TAG: &'static str = "Fields";
    }

    impl NodeTag for Newtype {
        const TAG: &'static str = "Newtype";
    }

    impl_node_set_serde! {
        dyn TestNode {
            Unit,
            Fields,
            Newtype,
        }
    }

    #[test]
    fn roundtrips_all_struct_shapes() {
        let fields = vec![("a", Datum::I64(3)), ("b", Datum::Str("hi".into()))];
        let cases = [
            node_datum("Unit", vec![]),
            node_datum("Fields", fields.clone()),
            node_datum("Newtype", fields),
        ];
        for datum in cases {
            let node: Box<dyn TestNode> = from_datum(datum.clone()).unwrap();
            let back = to_datum(&node).unwrap();
            // Serialization sorts map entries (`Datum`'s map serializer), so
            // compare the tag and the key-sorted entries.
            assert_eq!(back.get("type"), datum.get("type"));
            let (Datum::Map(mut a), Datum::Map(mut b)) = (back, datum) else {
                panic!("expected maps");
            };
            a.sort_by(|(k0, _), (k1, _)| k0.cmp(k1));
            b.sort_by(|(k0, _), (k1, _)| k0.cmp(k1));
            assert_eq!(a, b);
        }
    }

    #[test]
    fn nested_box_serializes_as_inner() {
        let node: Box<dyn TestNode> = Box::new(Unit);
        let nested: Box<dyn TestNode> = Box::new(node);
        assert_eq!(to_datum(&nested).unwrap(), node_datum("Unit", vec![]));
    }

    fn expect_err(result: Result<Box<dyn TestNode>, crate::DatumError>) -> String {
        match result {
            Ok(_) => panic!("expected a deserialization error"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn unknown_tag_names_the_expected_set() {
        let msg = expect_err(from_datum(node_datum("Mystery", vec![])));
        assert!(msg.contains("unknown variant `Mystery`"), "{msg}");
        assert!(msg.contains("`Unit`"), "{msg}");
    }

    #[test]
    fn missing_tag_is_a_missing_field() {
        let msg = expect_err(from_datum(Datum::Map(vec![])));
        assert!(msg.contains("missing field `type`"), "{msg}");
    }
}
