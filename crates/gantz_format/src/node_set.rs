//! Deterministic serde dispatch for `Box<dyn Node>`-style node sets.
//!
//! Each node type declares its wire tag at its own definition site via
//! [`gantz_nodetag::NodeTag`], usually derived. The tag is the `"type"` entry
//! of the node's serialized map. An application composes its node set with
//! [`impl_node_set_serde!`], which generates `Serialize` and `Deserialize`
//! for the erased `Box<dyn Trait>` as a compiled match over the listed types.
//!
//! There is no runtime registry. `typetag` registers types through
//! `inventory` constructors that run before `main`, and the WASM linker can
//! silently discard those. A compiled match cannot be dropped at link time.

/// The tag-first map wrapper the generated `Serialize` uses. `flatten` forces
/// `serialize_map`, which reproduces the `typetag` wire shape. A unit-struct
/// node flattens to nothing and leaves a tag-only map.
///
/// Public for the macro expansion only. Not part of the crate's API.
#[doc(hidden)]
#[derive(serde::Serialize)]
pub struct TaggedNode<'a, T: serde::Serialize> {
    pub r#type: &'static str,
    #[serde(flatten)]
    pub node: &'a T,
}

/// Implement `Serialize` and `Deserialize` for a node set's `Box<dyn Trait>`
/// by dispatching on each listed type's [`gantz_nodetag::NodeTag`].
///
/// The serialized form is a map carrying the node's `TAG` under a `"type"`
/// entry alongside the node's own fields. It is `typetag`-compatible, but
/// dispatch compiles to a plain match with no runtime registry.
///
/// The trait must have [`std::any::Any`] as a direct or transitive supertrait,
/// and the calling crate must depend on `serde`. To add a node type, derive or
/// implement [`gantz_nodetag::NodeTag`] beside the type, then add one line
/// here. A round-trip gate test over the full node set guards against a
/// missing line.
///
/// ```
/// use gantz_nodetag::NodeTag;
///
/// trait Node: std::any::Any {}
///
/// #[derive(serde::Serialize, serde::Deserialize, NodeTag)]
/// struct Gain {
///     db: f64,
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
                // A nested box delegates to the inner node's tag.
                // `Box<dyn Trait>` typically implements the trait itself.
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
                struct NodeSetVisitor;

                impl<'de> ::serde::de::Visitor<'de> for NodeSetVisitor {
                    type Value = ::std::boxed::Box<dyn $trait_>;

                    fn expecting(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                        f.write_str(::std::concat!(
                            "a `type`-tagged map for `dyn ",
                            ::std::stringify!($trait_),
                            "`",
                        ))
                    }

                    fn visit_map<A>(self, mut map: A) -> ::std::result::Result<Self::Value, A::Error>
                    where
                        A: ::serde::de::MapAccess<'de>,
                    {
                        const EXPECTED: &[&str] = &[$(<$ty as $crate::NodeTag>::TAG),+];
                        // Any fields seen before the tag, buffered as datums.
                        let mut entries: ::std::vec::Vec<(::std::string::String, $crate::Datum)> =
                            ::std::vec::Vec::new();
                        while let ::std::option::Option::Some(key) =
                            map.next_key::<::std::string::String>()?
                        {
                            if key != "type" {
                                entries.push((key, map.next_value()?));
                                continue;
                            }
                            let tag: ::std::string::String = map.next_value()?;
                            if entries.is_empty() {
                                // The layout the format writes. The tag leads,
                                // so the remaining fields stream typed
                                // straight into the node's `Deserialize`.
                                $(
                                    if tag == <$ty as $crate::NodeTag>::TAG {
                                        let node = <$ty as ::serde::Deserialize>::deserialize(
                                            $crate::NodeFields::new(map),
                                        )?;
                                        return ::std::result::Result::Ok(::std::boxed::Box::new(
                                            node,
                                        )
                                            as ::std::boxed::Box<dyn $trait_>);
                                    }
                                )+
                            } else {
                                // The tag arrived late. Buffer the rest and
                                // replay the whole map through the codec.
                                while let ::std::option::Option::Some(entry) =
                                    map.next_entry()?
                                {
                                    entries.push(entry);
                                }
                                $(
                                    if tag == <$ty as $crate::NodeTag>::TAG {
                                        return $crate::from_datum::<$ty>($crate::Datum::Map(
                                            entries,
                                        ))
                                        .map(|node| {
                                            ::std::boxed::Box::new(node)
                                                as ::std::boxed::Box<dyn $trait_>
                                        })
                                        .map_err(::serde::de::Error::custom);
                                    }
                                )+
                            }
                            return ::std::result::Result::Err(
                                ::serde::de::Error::unknown_variant(&tag, EXPECTED),
                            );
                        }
                        ::std::result::Result::Err(::serde::de::Error::missing_field("type"))
                    }
                }

                deserializer.deserialize_map(NodeSetVisitor)
            }
        }
    };
}

/// The deserializer for a node's fields once the leading `type` tag is
/// consumed. The remaining map entries stream directly into the concrete
/// node's `Deserialize`. This preserves the format's own typed handling, such
/// as RON's newtype syntax, rather than flattening it through a buffer.
///
/// Public for the macro expansion only. Not part of the crate's API.
#[doc(hidden)]
pub struct NodeFields<A> {
    map: StringKeys<A>,
}

impl<A> NodeFields<A> {
    pub fn new(map: A) -> Self {
        Self {
            map: StringKeys { map },
        }
    }
}

impl<'de, A> serde::Deserializer<'de> for NodeFields<A>
where
    A: serde::de::MapAccess<'de>,
{
    type Error = A::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_map(self.map)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_unit()
    }

    /// A unit-struct node has no fields beyond the tag.
    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_unit()
    }

    /// A newtype node such as `Fn<N>` shares its map with the wrapped node.
    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option seq tuple tuple_struct map struct enum
        identifier ignored_any
    }
}

/// Presents map keys as plain strings however the key seed asks for them. A
/// derived struct's field visitor requests `deserialize_identifier`. Some
/// formats, such as RON, only honour that in their native struct syntax and
/// not inside the `{...}` map a tagged node is written as.
struct StringKeys<A> {
    map: A,
}

impl<'de, A> serde::de::MapAccess<'de> for StringKeys<A>
where
    A: serde::de::MapAccess<'de>,
{
    type Error = A::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: serde::de::DeserializeSeed<'de>,
    {
        self.map.next_key_seed(StringKeySeed { seed })
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        self.map.next_value_seed(seed)
    }

    fn size_hint(&self) -> Option<usize> {
        self.map.size_hint()
    }
}

struct StringKeySeed<K> {
    seed: K,
}

impl<'de, K> serde::de::DeserializeSeed<'de> for StringKeySeed<K>
where
    K: serde::de::DeserializeSeed<'de>,
{
    type Value = K::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        self.seed.deserialize(StringKeyDeserializer {
            delegate: deserializer,
        })
    }
}

struct StringKeyDeserializer<D> {
    delegate: D,
}

impl<'de, D> serde::Deserializer<'de> for StringKeyDeserializer<D>
where
    D: serde::Deserializer<'de>,
{
    type Error = D::Error;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        self.delegate.deserialize_str(visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use crate::{Datum, from_datum, node_datum, to_datum};
    use gantz_nodetag::NodeTag;

    /// The dispatch handles the unit, fields and newtype node struct shapes.
    /// A newtype such as `Fn<N>` delegates to the wrapped node's fields.
    /// `Newtype` also exercises the derive's `#[tag(..)]` override. The others
    /// take the type-name default.
    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, NodeTag)]
    struct Unit;

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, NodeTag)]
    struct Fields {
        a: i64,
        b: String,
    }

    #[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, NodeTag)]
    #[tag("test.newtype")]
    struct Newtype(Fields);

    trait TestNode: std::any::Any {}

    impl TestNode for Unit {}
    impl TestNode for Fields {}
    impl TestNode for Newtype {}
    impl TestNode for Box<dyn TestNode> {}

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
            node_datum("test.newtype", fields),
        ];
        for datum in cases {
            let node: Box<dyn TestNode> = from_datum(datum.clone()).unwrap();
            let back = to_datum(&node).unwrap();
            // `Datum`'s map serializer sorts map entries, so compare the tag
            // and the key-sorted entries.
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

    /// Core nodes through a foreign self-describing format. JSON exercises the
    /// streamed `NodeFields` path outside the `Datum` codec. `Branch`'s
    /// validating manual `Deserialize` composes as a trait object. The `Push`
    /// and `Pull` eval wrappers keep their behaviour through typed round-trips.
    mod core_nodes {
        use gantz_core::node::{
            self, Branch, Conns, Expr, MetaCtx, Node, Pull, Push, WithPullEval, WithPushEval,
        };

        trait SerdeNode: Node {}

        impl SerdeNode for Branch {}
        impl SerdeNode for Expr {}

        crate::impl_node_set_serde! {
            dyn SerdeNode {
                Branch,
                Expr,
            }
        }

        fn no_lookup(_: &gantz_ca::ContentAddr) -> Option<&'static dyn Node> {
            None
        }

        #[test]
        fn eval_wrappers_roundtrip_through_json() {
            let expr = || node::expr("(+ $a $b)").unwrap();
            let ctx = MetaCtx::new(&no_lookup);

            // The plain expr through the erased node set.
            let boxed: Box<dyn SerdeNode> = Box::new(expr());
            let json = serde_json::to_string(&boxed).expect("serialize");
            let node: Box<dyn SerdeNode> = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(node.n_inputs(ctx), 2);
            assert_eq!(node.n_outputs(ctx), 1);
            assert!(node.push_eval(ctx).is_empty());
            assert!(node.pull_eval(ctx).is_empty());

            // The eval wrappers, typed. The orphan rule means only `gantz_core`
            // could tag the foreign `Push<Expr>` and `Pull<Expr>`, and no
            // production node set registers them. The app's gate test covers
            // the erased dimension with `Fn<NamedRef>` instead.
            let json = serde_json::to_string(&expr().with_push_eval()).expect("serialize");
            let push: Push<Expr> = serde_json::from_str(&json).expect("deserialize");
            assert!(!push.push_eval(ctx).is_empty());
            assert!(push.pull_eval(ctx).is_empty());
            let json = serde_json::to_string(&expr().with_pull_eval()).expect("serialize");
            let pull: Pull<Expr> = serde_json::from_str(&json).expect("deserialize");
            assert!(pull.push_eval(ctx).is_empty());
            assert!(!pull.pull_eval(ctx).is_empty());
        }

        #[test]
        fn branch_roundtrips_through_json() {
            let branch = Branch::new(
                "(if (equal? 0 $x) (list 0 $x) (list 1 $x))",
                vec![
                    Conns::try_from([true, false]).unwrap(),
                    Conns::try_from([false, true]).unwrap(),
                ],
            )
            .unwrap();

            let boxed: Box<dyn SerdeNode> = Box::new(branch);
            let json = serde_json::to_string(&boxed).expect("serialize");
            let node: Box<dyn SerdeNode> = serde_json::from_str(&json).expect("deserialize");

            let ctx = MetaCtx::new(&no_lookup);
            assert_eq!(node.n_inputs(ctx), 1);
            assert_eq!(node.n_outputs(ctx), 2);

            let branches = node.branches(ctx);
            assert_eq!(branches.len(), 2);
            assert_eq!(
                branches[0],
                node::EvalConf::Set(Conns::try_from([true, false]).unwrap()),
            );
            assert_eq!(
                branches[1],
                node::EvalConf::Set(Conns::try_from([false, true]).unwrap()),
            );
        }
    }
}
