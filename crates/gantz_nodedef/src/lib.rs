//! Node types defined as data.
//!
//! A domain defines a node type with a Rust type whose serde form is the map of
//! the stored fields of its nodes. The type implements [`NodeDef`], whose
//! methods are pure functions of the node. So a host decodes a node from its
//! fields once per change, and keeps what the node gives: its [`Meta`] and its
//! [`View`]. [`of`] gives the definition to a host in its object-safe form, a
//! [`DynNodeDef`].
//!
//! The crate depends on neither Steel nor egui. Fields, the values of edits and
//! the state of a node are [`Datum`]s, and views are `gantz_ui` elements.

pub use gantz_ca::Datum;

use gantz_ca::ContentAddr;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::any::Any;
use std::marker::PhantomData;
use std::sync::Arc;

/// A node of a type that a domain defines as data. See the crate docs.
pub trait NodeDef: Serialize + DeserializeOwned + Send + Sync + 'static {
    /// The tag of the stored form of the nodes. It is unique across the node
    /// types of a node set.
    const TAG: &'static str;

    /// The shape and the requirements of the node.
    fn meta(&self) -> Meta;

    /// The Steel expr that gives the outputs of the node from its inputs.
    ///
    /// When the node has more than one output, the expr gives a list with
    /// one value per output.
    fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String>;

    /// How the host shows the node.
    fn view(&self) -> View {
        View::default()
    }

    /// The node after an edit that a control of the view sends, with the new
    /// value of the control, or `None` when the edit changes nothing. A button
    /// sends [`Datum::Null`]. The host decodes the edited node again, which
    /// checks it.
    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let _ = (edit, value);
        None
    }

    /// The state of a new node, when [`Meta::stateful`] is true.
    fn init_state(&self) -> Option<Datum> {
        None
    }

    /// The new state of the node, given the state that it has, or `None` to
    /// keep it. The host asks each time it registers a node that has state,
    /// such as after an edit of the node. A node whose state follows its
    /// fields gives its new state here.
    fn sync_state(&self, state: &Datum) -> Option<Datum> {
        let _ = state;
        None
    }
}

/// A node definition, as a host calls it. Each method takes the fields of a node.
/// [`of`] gives the definition of a [`NodeDef`].
pub trait DynNodeDef: Send + Sync {
    /// See [`NodeDef::TAG`].
    fn tag(&self) -> &str;

    /// Decodes a node from its fields, and describes it.
    fn describe(&self, fields: &Datum) -> Result<Description, String>;

    /// The Steel expr of a node. See [`NodeDef::expr`].
    fn expr(&self, fields: &Datum, ctx: ExprArgs<'_>) -> Result<String, String>;

    /// The fields of a node after an edit, or `None` when the edit changes
    /// nothing. See [`NodeDef::edit`].
    fn edit(&self, fields: &Datum, edit: &str, value: &Datum) -> Result<Option<Datum>, String>;

    /// The state of a node, given the state that it has if any, or `None` to
    /// keep it. See [`NodeDef::init_state`] and [`NodeDef::sync_state`].
    fn state(&self, fields: &Datum, state: Option<&Datum>) -> Option<Datum>;
}

/// What a host keeps of a node.
pub struct Description {
    /// The fields of the decoded node, as the definition encodes them.
    pub fields: Datum,
    pub meta: Meta,
    pub view: View,
    /// The decoded node, for code in the process of the definition that reads
    /// nodes by their type, such as the compiler of a domain.
    pub node: Arc<dyn Any + Send + Sync>,
}

/// The shape and the requirements of a node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    /// The name that the host shows, such as `pat.mini`. The tag when empty.
    pub name: String,
    /// What the node does, for the node palette.
    pub description: String,
    pub inputs: usize,
    pub outputs: usize,
    /// Whether the expr reads and writes its `state`.
    pub stateful: bool,
    /// Whether the node has a push entrypoint.
    pub push: bool,
    /// Whether the node has a pull entrypoint.
    pub pull: bool,
    /// The names of the Steel modules that the expr uses.
    pub requires: Vec<String>,
    /// The possible branches of the node, each the outputs that it enables.
    /// Empty for a node that always enables every output. With branches, the
    /// expr gives a list of the index of the branch and the values of the
    /// outputs.
    pub branches: Vec<Vec<bool>>,
    /// The blobs that the node uses, which live while it does, each by the
    /// section of the registry that holds it and its address.
    pub blobs: Vec<(String, ContentAddr)>,
}

/// How a host shows a node.
///
/// The bind paths of the trees start from the node itself, so `(bind ())` is
/// the state of the node. Their controls edit the node through its fields or
/// through [`NodeDef::edit`].
#[derive(Clone, Debug, Default)]
pub struct View {
    /// The docs of the inputs, in socket order.
    pub inputs: Vec<SocketDoc>,
    /// The docs of the outputs, in socket order.
    pub outputs: Vec<SocketDoc>,
    /// The body of the node. Without one, the host shows the name.
    pub body: Option<gantz_ui::Element>,
    /// How the body resizes, if it does.
    pub resize: Option<Resize>,
    /// The rows of the table of the node inspector.
    pub rows: Vec<Row>,
    /// A tree that the node inspector shows below its table.
    pub inspector: Option<gantz_ui::Element>,
    /// Whether the inspector leaves out the raw state of the node, for a node
    /// whose rows show its state better.
    pub hide_state: bool,
    /// Whether the body fills the pane of the node edge to edge.
    pub fill: bool,
    /// The items of the context menu of the node.
    pub menu: Vec<MenuItem>,
}

/// How the body of a node resizes. The host sets the two fields, as integers,
/// when a resize settles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resize {
    /// The field that holds the width of the body in points.
    pub width: String,
    /// The field that holds the height of the body in points.
    pub height: String,
    /// Whether the body insets its content within the margin of the frame.
    pub margin: bool,
    /// The size of the body while the fields hold no valid size.
    pub default: [u16; 2],
}

/// A row of the table of the node inspector.
#[derive(Clone, Debug)]
pub struct Row {
    pub label: String,
    pub ui: gantz_ui::Element,
    /// The height of the row in lines of the table, at least one.
    pub lines: u32,
}

/// The doc of an input or an output of a node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SocketDoc {
    /// The type of the values, such as `number`.
    pub ty: String,
    /// What the values mean.
    pub description: String,
}

/// An item of the context menu of a node. It sends its edit with no value,
/// with a file that an importer reads, or with the new state of a checkbox.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// Text shown while the pointer hovers the item.
    pub tip: String,
    /// The edit. See [`NodeDef::edit`].
    pub edit: String,
    /// The name of the importer of the file that the edit takes, as a `(file)`
    /// control names it.
    pub import: Option<String>,
    /// The state of a checkbox item. A click sends the edit with the new
    /// state.
    pub checked: Option<bool>,
}

/// What the expr of a node depends on, beside the node.
#[derive(Clone, Copy, Debug)]
pub struct ExprArgs<'a> {
    /// The Steel name of each input value, or `None` for an input with no
    /// connection.
    pub inputs: &'a [Option<&'a str>],
    /// Whether each output has a connection.
    pub outputs: &'a [bool],
    /// The path of the node from the root graph of its head.
    pub path: &'a [usize],
    /// The Steel name of the map of the arguments of the evaluation, such as
    /// its `time`.
    pub args: &'a str,
}

/// The [`DynNodeDef`] of a [`NodeDef`]. See [`of`].
struct Typed<D>(PhantomData<fn() -> D>);

impl SocketDoc {
    /// The doc of a socket of values of type `ty`.
    pub fn new(ty: &str, description: &str) -> Self {
        SocketDoc {
            ty: ty.into(),
            description: description.into(),
        }
    }
}

impl<D: NodeDef> DynNodeDef for Typed<D> {
    fn tag(&self) -> &str {
        D::TAG
    }

    fn describe(&self, fields: &Datum) -> Result<Description, String> {
        let node = decode::<D>(fields)?;
        Ok(Description {
            fields: encode(&node)?,
            meta: node.meta(),
            view: node.view(),
            node: Arc::new(node),
        })
    }

    fn expr(&self, fields: &Datum, ctx: ExprArgs<'_>) -> Result<String, String> {
        decode::<D>(fields)?.expr(ctx)
    }

    fn edit(&self, fields: &Datum, edit: &str, value: &Datum) -> Result<Option<Datum>, String> {
        match decode::<D>(fields)?.edit(edit, value) {
            Some(node) => Ok(Some(encode(&node)?)),
            None => Ok(None),
        }
    }

    fn state(&self, fields: &Datum, state: Option<&Datum>) -> Option<Datum> {
        let node = decode::<D>(fields).ok()?;
        match state {
            Some(state) => node.sync_state(state),
            None => node.init_state(),
        }
    }
}

/// The definition of the node type `D`.
pub fn of<D: NodeDef>() -> Arc<dyn DynNodeDef> {
    Arc::new(Typed::<D>(PhantomData))
}

/// The fields of `node`, the map of its serde form. A unit struct has no
/// fields.
pub fn encode<T: Serialize>(node: &T) -> Result<Datum, String> {
    match gantz_ca::datum::to_datum(node).map_err(|e| e.to_string())? {
        Datum::Map(mut fields) => {
            // As in `gantz_core::data::erase_node_tagged`, the tag of the
            // definition is the one source of the tag, so a serde `"type"` entry goes.
            fields.retain(|(k, _)| k != "type");
            Ok(Datum::Map(fields))
        }
        Datum::Null => Ok(Datum::Map(vec![])),
        _ => Err("the serde form of a node is not a map".into()),
    }
}

/// The stored form of `node`: its fields under its tag, with the blobs that it
/// uses.
pub fn node_data<D: NodeDef>(node: &D) -> Result<gantz_ca::NodeData, String> {
    let mut data = gantz_ca::NodeData::new(D::TAG, encode(node)?);
    data.blobs = node.meta().blobs;
    data.canonicalize();
    Ok(data)
}

/// The node of type `D` with the fields `fields`.
pub fn decode<D: NodeDef>(fields: &Datum) -> Result<D, String> {
    gantz_ca::datum::from_datum(fields.clone()).map_err(|e| format!("{}: {e}", D::TAG))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// A node that scales its input by `gain` and keeps `gain` as its state.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Gain {
        gain: i64,
    }

    impl NodeDef for Gain {
        const TAG: &'static str = "Gain";

        fn meta(&self) -> Meta {
            Meta {
                inputs: 1,
                outputs: 1,
                stateful: true,
                ..Meta::default()
            }
        }

        fn expr(&self, ctx: ExprArgs<'_>) -> Result<String, String> {
            let x = ctx.inputs[0].ok_or("no input")?;
            Ok(format!("(* {x} {})", self.gain))
        }

        fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
            match (edit, value) {
                ("gain", Datum::I64(gain)) => Some(Gain { gain: *gain }),
                _ => None,
            }
        }

        fn init_state(&self) -> Option<Datum> {
            Some(Datum::I64(self.gain))
        }

        fn sync_state(&self, state: &Datum) -> Option<Datum> {
            (*state != Datum::I64(self.gain)).then_some(Datum::I64(self.gain))
        }
    }

    fn fields(gain: i64) -> Datum {
        Datum::Map(vec![("gain".into(), Datum::I64(gain))])
    }

    /// A definition decodes a node from its fields, and describes it with its
    /// own encoding of the fields and the decoded node.
    #[test]
    fn a_def_describes_a_node_of_its_fields() {
        let def = of::<Gain>();
        assert_eq!(def.tag(), "Gain");
        let desc = def.describe(&fields(3)).unwrap();
        assert_eq!(desc.fields, fields(3));
        assert_eq!(desc.meta.inputs, 1);
        assert_eq!(desc.node.downcast_ref(), Some(&Gain { gain: 3 }));
        let err = def.describe(&Datum::Map(vec![])).err().unwrap();
        assert!(err.starts_with("Gain: "), "{err}");
    }

    /// The expr, the edits and the state follow the fields.
    #[test]
    fn a_def_gives_the_expr_edits_and_state_of_a_node() {
        let def = of::<Gain>();
        let args = ExprArgs {
            inputs: &[Some("x")],
            outputs: &[true],
            path: &[0],
            args: "args",
        };
        assert_eq!(def.expr(&fields(2), args).unwrap(), "(* x 2)");
        let edited = def.edit(&fields(2), "gain", &Datum::I64(5)).unwrap();
        assert_eq!(edited, Some(fields(5)));
        assert_eq!(def.edit(&fields(2), "other", &Datum::Null).unwrap(), None);
        assert_eq!(def.state(&fields(2), None), Some(Datum::I64(2)));
        assert_eq!(def.state(&fields(2), Some(&Datum::I64(2))), None);
        assert_eq!(
            def.state(&fields(2), Some(&Datum::I64(1))),
            Some(Datum::I64(2))
        );
    }
}
