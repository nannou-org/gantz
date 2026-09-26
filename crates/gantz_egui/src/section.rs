//! The GUI's registry metadata sections: graph descriptions, demo
//! associations and per-commit scene views.
//!
//! Each section is declared via [`gantz_ca::SectionDecl`], so the data rides
//! the registry through merge, prune, export and the `.gantz` text format.
//! See [`crate::format`] for the friendly text forms.

use crate::SceneView;
use gantz_ca::{
    CommitAddr, Liveness, MergePolicy, Name, Registry, SectionDecl, section_get, section_insert,
    section_iter, section_remove,
};

/// Human-readable descriptions for named graphs.
pub struct Descriptions;

/// Demo associations: a graph name mapped to its demo graph's name.
///
/// Keyed by name rather than commit so the association survives an edit.
/// Editing a graph mints a new commit but keeps the name.
pub struct Demos;

/// Persisted scene views, keyed by commit. A view is a camera plus a node
/// layout.
pub struct Views;

/// The id of the [`Descriptions`] section.
pub const DESCRIPTIONS_ID: &str = "gantz.description";

/// The id of the [`Demos`] section.
pub const DEMOS_ID: &str = "egui.demo";

/// The id of the [`Views`] section.
pub const VIEWS_ID: &str = "egui.view";

impl SectionDecl for Descriptions {
    const ID: &'static str = DESCRIPTIONS_ID;
    const POLICY: MergePolicy = MergePolicy::KeepExisting;
    const LIVENESS: Liveness = Liveness::WithName;
    type Key = Name;
    type Value = String;
}

impl SectionDecl for Demos {
    const ID: &'static str = DEMOS_ID;
    const POLICY: MergePolicy = MergePolicy::KeepExisting;
    const LIVENESS: Liveness = Liveness::WithName;
    type Key = Name;
    type Value = String;
}

impl SectionDecl for Views {
    const ID: &'static str = VIEWS_ID;
    const POLICY: MergePolicy = MergePolicy::KeepExisting;
    const LIVENESS: Liveness = Liveness::WithCommit;
    type Key = CommitAddr;
    type Value = SceneView;
}

/// The stored description for the named graph, if any.
pub fn description(reg: &Registry, name: &Name) -> Option<String> {
    section_get::<Descriptions>(reg, name)
}

/// Store a description for the named graph. An empty string removes the
/// entry.
pub fn set_description(reg: &mut Registry, name: Name, description: String) {
    if description.is_empty() {
        section_remove::<Descriptions>(reg, &name);
    } else {
        section_insert::<Descriptions>(reg, name, &description)
            .expect("a `String` always encodes as a datum");
    }
}

/// All stored descriptions, in name order.
pub fn descriptions(reg: &Registry) -> impl Iterator<Item = (Name, String)> + '_ {
    section_iter::<Descriptions>(reg)
}

/// The demo graph name associated with the named graph, if any.
pub fn demo(reg: &Registry, name: &Name) -> Option<String> {
    section_get::<Demos>(reg, name)
}

/// Associate the named graph with the given demo graph name.
pub fn set_demo(reg: &mut Registry, name: Name, demo: String) {
    section_insert::<Demos>(reg, name, &demo).expect("a `String` always encodes as a datum");
}

/// Remove the named graph's demo association.
pub fn remove_demo(reg: &mut Registry, name: &Name) -> Option<String> {
    section_remove::<Demos>(reg, name)
}

/// All demo associations, in name order.
pub fn demos(reg: &Registry) -> impl Iterator<Item = (Name, String)> + '_ {
    section_iter::<Demos>(reg)
}

/// The stored scene view for the given commit, if any.
pub fn view(reg: &Registry, ca: &CommitAddr) -> Option<SceneView> {
    section_get::<Views>(reg, ca)
}

/// Store a scene view for the given commit.
pub fn set_view(reg: &mut Registry, ca: CommitAddr, view: &SceneView) {
    if let Err(e) = section_insert::<Views>(reg, ca, view) {
        log::error!("failed to encode scene view for {ca}: {e}");
    }
}

/// Seed `ca`'s stored view, ignoring empty layouts. An empty layout reads as
/// never laid out to the scene, which then auto-layouts destructively, so it
/// must never become a commit's baseline.
pub fn seed_view(reg: &mut Registry, ca: CommitAddr, view: &SceneView) {
    if !view.layout.is_empty() {
        set_view(reg, ca, view);
    }
}

/// Remove the stored scene view for the given commit.
pub fn remove_view(reg: &mut Registry, ca: &CommitAddr) -> Option<SceneView> {
    section_remove::<Views>(reg, ca)
}

/// All stored scene views, in commit order.
pub fn views(reg: &Registry) -> impl Iterator<Item = (CommitAddr, SceneView)> + '_ {
    section_iter::<Views>(reg)
}
