//! Export/import representation for sharing node sets between gantz instances.
//!
//! The [`Export`] type bundles a [`gantz_ca::Registry`] subset with optional
//! [`GraphViews`] layout data. Serialization uses RON with the `.gantz` file
//! extension.

use crate::GraphViews;
use gantz_ca::{CommitAddr, registry::MergeResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A serializable bundle of a registry subset and its associated view state.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Export<G> {
    pub registry: gantz_ca::Registry<G>,
    #[serde(default)]
    pub views: HashMap<CommitAddr, GraphViews>,
}

/// Produce an [`Export`] by filtering views to commits present in the registry.
pub fn export_with_views<G>(
    registry: gantz_ca::Registry<G>,
    all_views: &HashMap<CommitAddr, GraphViews>,
) -> Export<G>
where
    G: Clone,
{
    let views = all_views
        .iter()
        .filter(|(ca, _)| registry.commits().contains_key(ca))
        .map(|(&ca, v)| (ca, v.clone()))
        .collect();
    Export { registry, views }
}

/// Merge an [`Export`] into an existing registry and views map.
///
/// Incoming views for new commits are inserted; existing views for known
/// commits are kept.
pub fn merge_with_views<G>(
    registry: &mut gantz_ca::Registry<G>,
    views: &mut HashMap<CommitAddr, GraphViews>,
    export: Export<G>,
) -> MergeResult {
    let result = registry.merge(export.registry);
    for (ca, v) in export.views {
        views.entry(ca).or_insert(v);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_ca::{Commit, ContentAddr};
    use std::{collections::BTreeMap, time::Duration};

    fn graph_addr(n: u8) -> gantz_ca::GraphAddr {
        gantz_ca::GraphAddr::from(ContentAddr::from([n; 32]))
    }

    fn commit_addr_raw(n: u8) -> CommitAddr {
        CommitAddr::from(ContentAddr::from([n; 32]))
    }

    fn test_export() -> Export<String> {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        let registry = gantz_ca::Registry::new(
            HashMap::from([(ga, "graph_a".to_string())]),
            HashMap::from([(ca, commit)]),
            BTreeMap::from([("alpha".to_string(), ca)]),
        );
        Export {
            registry,
            views: HashMap::new(),
        }
    }

    #[test]
    fn round_trip_serde() {
        let export = test_export();
        let s = ron::to_string(&export).expect("serialize");
        let recovered: Export<String> = ron::from_str(&s).expect("deserialize");
        assert_eq!(
            export.registry.commits().len(),
            recovered.registry.commits().len()
        );
        assert_eq!(
            export.registry.graphs().len(),
            recovered.registry.graphs().len()
        );
        assert_eq!(
            export.registry.names().len(),
            recovered.registry.names().len()
        );
        // Verify the content survived the round-trip.
        let ca = commit_addr_raw(10);
        assert!(recovered.registry.commits().contains_key(&ca));
        assert_eq!(recovered.registry.names().get("alpha"), Some(&ca));
    }

    #[test]
    fn round_trip_export_merge_recovers_data() {
        let export = test_export();
        let s = ron::to_string(&export).expect("serialize");
        let recovered: Export<String> = ron::from_str(&s).expect("deserialize");
        let mut target = gantz_ca::Registry::<String>::default();
        let mut views = HashMap::new();
        let result = merge_with_views(&mut target, &mut views, recovered);
        assert_eq!(result.names_added, vec!["alpha".to_string()]);
        assert!(result.names_replaced.is_empty());
        let ca = commit_addr_raw(10);
        assert!(target.commits().contains_key(&ca));
        assert_eq!(target.names().get("alpha"), Some(&ca));
    }

    #[test]
    fn export_with_views_filters_views() {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let cb = commit_addr_raw(20);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        let registry = gantz_ca::Registry::new(
            HashMap::from([(ga, "g".to_string())]),
            HashMap::from([(ca, commit)]),
            BTreeMap::new(),
        );
        let mut all_views = HashMap::new();
        all_views.insert(ca, GraphViews::new());
        all_views.insert(cb, GraphViews::new()); // cb not in registry
        let export = export_with_views(registry, &all_views);
        assert!(export.views.contains_key(&ca));
        assert!(!export.views.contains_key(&cb));
    }

    #[test]
    fn merge_with_views_keeps_existing_views() {
        let ga = graph_addr(1);
        let ca = commit_addr_raw(10);
        let commit = Commit::new(Duration::from_secs(1), None, ga);
        let mut registry = gantz_ca::Registry::new(
            HashMap::from([(ga, "g".to_string())]),
            HashMap::from([(ca, commit.clone())]),
            BTreeMap::new(),
        );
        let mut existing_view = GraphViews::new();
        existing_view.insert(vec![0], egui_graph::View::default());
        let mut views = HashMap::from([(ca, existing_view)]);
        let export = Export {
            registry: gantz_ca::Registry::new(
                HashMap::from([(ga, "g".to_string())]),
                HashMap::from([(ca, commit)]),
                BTreeMap::new(),
            ),
            views: HashMap::from([(ca, GraphViews::new())]),
        };
        merge_with_views(&mut registry, &mut views, export);
        // Existing view (with 1 entry) should be preserved, not replaced by empty.
        assert_eq!(views[&ca].len(), 1);
    }
}
