//! The declarative UI tree model for user-defined gantz GUIs.
//!
//! A graph's GUI is a value. It is a tree of plain data that the graph
//! itself produces. The tree binds to node state by path. A host interprets
//! it every frame. This crate defines the typed model of that tree
//! ([`Element`]), a total decoder with inline diagnostics, a canonical
//! encoder, and a Steel codec. It depends on no GUI toolkit. With no features
//! enabled it depends on no runtime.
//!
//! # The form
//!
//! The vocabulary is specified over [`Datum`](gantz_ca::Datum), the data form
//! of gantz values. The tree uses its symbols, booleans, integers, floats,
//! strings and sequences. The reference syntax is s-expressions:
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
//! Tags and names are symbols. The decoder treats symbols and strings as
//! interchangeable in identifier and text positions.
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
//! | `(dialer)` | number | `bind`, `min`, `max`, `step` (numbers), `precision` (int), `label`, `prefix`, `suffix` (strings), `push` (bool, default `#t`), `style` (reserved: `slider`/`knob`) |
//! | `(toggle)` | bool | `bind`, `label`, `push` (default `#t`) |
//! | `(choice)` | one of its options | `bind`, `options` (list of symbols) |
//! | `(text-input)` | string | `bind`, `hint` (string), `mono` (bool, default `#f`), `push` (default `#t`). An edit commits on focus loss, on enter or once typing pauses |
//! | `(bound)` | number or `#f` | `bind`, `label` (string). A checkbox sets the state to the number beside it, or clears it to `#f` |
//! | `(button)` | none | `bind`, `label`. A press queues a push eval, bang semantics stay in the node's expression |
//! | `(file)` | none, it imports into the node that owns the tree | `on` (edit), `import` (an importer's name), `label`, `tip`. A press picks a file for the host's importer. See [`File`] |
//! | `(matrix)` | list of rows of bool or number cells | `bind`, `cell-size` (number), `push` (default `#t`). Rows and columns come from the state shape, not attrs |
//! | `(envelope)` | none, it edits the node that owns the tree | `on` (edit), `value` (an envelope, `(init ((level time shape curve) ...) release)`), `x-range`, `y-range` (lists of two numbers), `grid`, `axes` (bools, default `#f`), `w`, `h` (numbers). See [`EnvelopeElem`] |
//!
//! A `dialer`, `toggle`, `text-input`, `button` or `color` takes a `tip`
//! (string) that shows while the pointer hovers the control. A `choice`
//! takes `tips` (list of strings), one per option. A `color` takes `theme`
//! (bool, default `#t`), whether a button shows that clears the colour back
//! to the theme's colour.
//!
//! ## Display
//!
//! | element | state shape | attrs |
//! |---|---|---|
//! | `(label "text")` | none | positional text (required), `size` (number), `color` (`"#rrggbb"` or `"#rrggbbaa"`), `tone` (`weak`/`strong`/`warn`/`error`, a theme colour used without `color`), `mono` (bool, default `#f`), `tip` (string) |
//! | `(value)` | any, rendered as its representation. A figure shows its summary | `bind`, `wrap` (bool, default `#f`) |
//! | `(plot)` | scope buffer or signal, a list of lists renders as stacked channels | `bind`, `mode` (`scope`/`signal`), `style` (`bars`/`line`), `color`, `grid`, `axes`, `interactive` (bools, default `#f`), `y-min`, `y-max`, `w`, `h` (numbers) |
//! | `(canvas)` | a list of panels of figures, see [`figure`] | `bind`, `grid`, `axes`, `interactive` (bools, default `#f`), `w`, `h` (numbers, the available size when absent) |
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
//! `tabs`, `page`, `paint`, `image`, `dropdown`, `xy-pad`,
//! `meter`. See [`RESERVED_TAGS`].
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
//! A node can give a tree as its own view, and its controls can then edit
//! the node itself in one of two ways:
//!
//! - A `field` binds a top-level field of the node, which the host reads and
//!   sets. See [`Field`].
//! - An `on` edit, with a `value` to show, sends the edit to the node, which
//!   applies it. See [`Edit`].
//!
//! A control takes its `field` first, then its `on` edit, then its `bind`.
//!
//! A `dialer`, `toggle`, `choice`, `text-input`, `bound` or `color` with a
//! `bind` takes `at`, a list of keys into the bound state, such as
//! `(at freq value)`. The control reads and writes the value at those keys
//! of the nested hash maps, and keeps the other entries. A `label` or
//! `canvas` with a `bind` also takes `at`, and shows the value at those
//! keys. See [`At`].
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
//! - `steel` is the default feature. [`codec::steel`] decodes from and
//!   encodes to `SteelVal` through `gantz_core::datum`. Symbols are
//!   identifiers. Steel lists and vectors both read as lists.
//! - `steel` also provides the `gantz/ui` Steel module, see [`modules()`].
//!   Its `ui-elem` builds an element from optional inputs, dropping absent
//!   attributes and positionals, so base graphs can compose trees from
//!   nodes with one expr each.
//!
//! With no features enabled the crate is the pure model: [`Element`],
//! [`decode()`], [`encode()`] and the [`pretty()`] printer that renders any
//! datum as indented scheme text.
//!
//! The canonical encoding ([`encode()`]) emits the minimal form. Only
//! attributes that differ from an element's `Default` emit. Required
//! attributes and positional arguments always emit. Attributes emit in
//! field declaration order, with `key` last.

pub use decode::{Decoded, Limits, decode};
pub use diag::{ErrorReason, TreePath, Warning, WarningKind};
pub use elem::{
    ATTRS_MARKER, Align, At, BindPath, Bound, Button, Canvas, Choice, Col, Color, Dialer,
    DialerStyle, Edit, Element, EnvelopeElem, ErrorElem, Field, File, Frame, Grid, Key, Label,
    Matrix, Plot, PlotMode, PlotStyle, RESERVED_TAGS, RefGui, Rgba, Row, Scope, Sep, Space,
    TextInput, Toggle, Tone, Value,
};
pub use encode::encode;
pub use figure::{Figures, decode_figures};
pub use sexpr::{flat, pretty, summary};

pub mod codec;
pub mod decode;
pub mod diag;
pub mod elem;
pub mod encode;
pub mod envelope;
pub mod figure;
pub mod sexpr;
#[cfg(test)]
mod tests;

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
