//! The declarative UI tree model for user-defined gantz GUIs.
//!
//! A graph's GUI is a value. It is a tree of plain data that the graph
//! itself produces. The tree binds to node state by path. A host interprets
//! it every frame. This crate defines the typed model of that tree
//! ([`Element`]), a total decoder with inline diagnostics, a canonical
//! encoder, and per-runtime codecs. It depends on no GUI toolkit. With no
//! features enabled it depends on no runtime.
//!
//! # The form
//!
//! The vocabulary is specified over an abstract data model ([`SExpr`]). The
//! model has identifier atoms, booleans, integers, floats, strings and
//! lists. The reference syntax is s-expressions:
//!
//! ```text
//! element  := (tag attrs? arg ... child ...)
//! attrs    := (@ (name value) ...)
//! tag,name := identifier
//! value    := bool | int | float | string | list
//! ```
//!
//! `@` is the reserved attribute marker and invalid as a user tag. A small
//! example:
//!
//! ```text
//! (col
//!   (frame (@ (title "filter"))
//!     (row (dialer (@ (bind (2)) (min 20.0) (max 20000.0) (label "cutoff")))
//!          (dialer (@ (bind (3)) (min 0.1) (max 4.0) (label "q")))))
//!   (row (toggle (@ (bind (5)) (label "drive")))
//!        (button (@ (bind (7)) (label "ping"))))
//!   (ref-gui 9))
//! ```
//!
//! Each runtime encodes the model in its own value type through a codec.
//! See [`codec`]. The Steel encoding writes tags and names as symbols. The
//! `Datum` encoding writes them as strings. The decoder treats identifiers
//! and strings as interchangeable in identifier and text positions, so both
//! encodings agree.
//!
//! # Principles
//!
//! 1. The tree is inert data. No callbacks, no code. The graph produces it
//!    on change. The host interprets it every frame.
//! 2. Bindings are structural and machine produced. Codegen or the host
//!    bakes the paths. They are never hand authored in the normal flow.
//! 3. State granularity beats binding splicing. A dynamic collection is one
//!    widget bound to one structured state value, not many spliced bindings.
//! 4. Restore never fires. Widgets emit events only from user interaction.
//! 5. Unknown attributes are ignored for forward compatibility. Unknown tags
//!    are visible inline errors, never silently blank.
//!
//! # Element catalog
//!
//! Every element accepts a `key` attribute. It is a string or integer
//! identity override for children whose order can change at runtime. See
//! [Identity](#identity-and-keys).
//!
//! ## Layout
//!
//! | element | children | attrs |
//! |---|---|---|
//! | `(col ...)` / `(row ...)` | elements | `gap` (number), `align` (`start`/`center`/`end`) |
//! | `(grid (@ (cols n)) ...)` | elements, row-major | `cols` (required positive int), `gap` |
//! | `(frame ...)` | elements | `title` (string) |
//! | `(sep)` | none | |
//! | `(space n)` | none | positional numeric amount, optional |
//! | `(scope id ...)` | elements | positional node id, required. Prefixes binding paths, see below |
//!
//! ## Controls
//!
//! Controls bind to node state and emit events. A `set` writes the bound
//! state. When the `push` attribute is on, a `set` also queues a push eval
//! at the bound node.
//!
//! | element | state shape | attrs |
//! |---|---|---|
//! | `(dialer)` | number | `bind`, `min`, `max`, `step` (numbers), `precision` (int), `label` (string), `push` (bool, default `#t`), `style` (reserved: `slider`/`knob`) |
//! | `(toggle)` | bool | `bind`, `label`, `push` (default `#t`) |
//! | `(button)` | none | `bind`, `label`. A press queues a push eval, bang semantics stay in the node's expression |
//! | `(matrix)` | list of rows of bool or number cells | `bind`, `cell-size` (number), `push` (default `#t`). Rows and columns come from the state shape, not attrs |
//!
//! ## Display
//!
//! | element | state shape | attrs |
//! |---|---|---|
//! | `(label "text")` | none | positional text (required), `size` (number), `color` (`"#rrggbb"` or `"#rrggbbaa"`) |
//! | `(value)` | any, rendered as its representation | `bind`, `wrap` (bool, default `#f`) |
//! | `(plot)` | scope buffer or signal, a list of lists renders as stacked channels | `bind`, `mode` (`scope`/`signal`), `style` (`bars`/`line`), `color`, `grid`, `axes`, `interactive` (bools, default `#f`), `y-min`, `y-max`, `w`, `h` (numbers) |
//!
//! ## Embedding
//!
//! `(ref-gui id)` is a host resolved embed of a child instance's GUI. The
//! host resolves the instance's body marker, else its auto GUI, else a
//! label. It renders the result at the instance path inside an implicit
//! scope. The required positional id is the instance's node id in the
//! defining graph.
//!
//! ## Reserved
//!
//! Tag names reserved for later versions decode as visible errors:
//! `tabs`, `page`, `canvas`, `paint`, `image`, `dropdown`, `text-input`,
//! `xy-pad`, `meter`. See [`RESERVED_TAGS`].
//!
//! # Binding model
//!
//! A `bind` attribute holds a list of non-negative integers. It is a node
//! path relative to the graph the tree was defined in. See [`BindPath`],
//! which matches `gantz_core::node::Id` paths. Hosts resolve a binding as
//! `instance prefix ++ scope prefixes ++ bind path`. They read and write VM
//! runtime state at that path. `(scope id ...)` pushes `id` onto the prefix
//! for its subtree. Widget fragments bake their own node id at codegen.
//! Hosts insert scopes where a child's GUI crosses into a parent.
//!
//! # Identity and keys
//!
//! Host widget identity derives from the render root, the accumulated scope
//! prefixes and the structural tree position. The `key` attribute overrides
//! the position. Keys are required for children whose order can change at
//! runtime. Without a key, widget memory such as focus and drag state
//! follows the position rather than the child. Label text never contributes
//! to identity, so relabelling never resets widget state.
//!
//! # Diagnostics and totality
//!
//! [`decode()`] is total. One boundary rule applies. See [`diag`].
//!
//! - A subtree that cannot render meaningfully becomes an inline
//!   [`Element::Error`] that keeps its slot. Causes are unknown or reserved
//!   tags, non-element values in element position, a misplaced `@` block, a
//!   missing required positional argument and an exceeded limit. Siblings
//!   decode independently.
//! - Everything recoverable is a [`Warning`] and the element still renders.
//!   Unknown attributes are ignored. Mistyped attribute values fall back to
//!   their defaults. Duplicates keep the first value. Extra children of leaf
//!   elements are dropped.
//!
//! [`Limits`] caps nesting depth and element count. A pathological computed
//! tree stays a visible inline error rather than a stalled host.
//!
//! # Codecs and features
//!
//! - `steel` is the default feature. [`codec::steel`] is the backend
//!   encoding over `SteelVal`. Symbols are identifiers. Steel lists and
//!   vectors both read as lists.
//! - `datum` enables [`codec::datum`], the storage and interchange encoding
//!   over `gantz_core::datum::Datum`. `Datum` has no symbol variant, so
//!   identifiers normalize to strings on a store and reload. The result
//!   decodes to the identical tree.
//! - `steel` also provides the `gantz/ui` Steel module, see [`modules()`].
//!   Its `ui-elem` builds an element from optional inputs, dropping absent
//!   attributes and positionals, so base graphs can compose trees from
//!   nodes with one expr each.
//!
//! With no features enabled the crate is the pure model: [`SExpr`],
//! [`Element`], [`decode()`], [`encode()`] and the [`pretty()`] printer that
//! renders any [`SExpr`] as indented scheme text.
//!
//! The canonical encoding ([`encode()`]) emits the minimal form. Only
//! attributes that differ from an element's `Default` emit. Required
//! attributes and positional arguments always emit. Attributes emit in
//! field declaration order, with `key` last.

pub use decode::{Decoded, Limits, decode};
pub use diag::{ErrorReason, TreePath, Warning, WarningKind};
pub use elem::{
    ATTRS_MARKER, Align, BindPath, Button, Col, Dialer, DialerStyle, Element, ErrorElem, Frame,
    Grid, Key, Label, Matrix, Plot, PlotMode, PlotStyle, RESERVED_TAGS, RefGui, Rgba, Row, Scope,
    Sep, Space, Toggle, Value,
};
pub use encode::encode;
pub use sexpr::{SExpr, flat, pretty, summary};

pub mod codec;
pub mod decode;
pub mod diag;
pub mod elem;
pub mod encode;
pub mod sexpr;

/// The `gantz/ui` Steel module.
///
/// Register via `gantz_core::vm::new_engine` or the app's steel-module
/// collection, then `(require "gantz/ui")` to use. It provides `ui-elem`,
/// `ui-int` and `ui-id`, the helpers the element base graphs are built on.
#[cfg(feature = "steel")]
pub const MODULE: gantz_core::vm::SteelModule =
    gantz_core::vm::SteelModule::new("gantz/ui", include_str!("ui.scm"));

/// The Steel modules provided by this crate.
#[cfg(feature = "steel")]
pub fn modules() -> &'static [gantz_core::vm::SteelModule] {
    const MODULES: &[gantz_core::vm::SteelModule] = &[MODULE];
    MODULES
}
