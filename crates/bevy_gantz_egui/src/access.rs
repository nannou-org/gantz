//! The `gantz_egui::HeadAccess` adapter over the open-head entities.

use crate::{GraphView, HeadNodeInstances};
use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryData;
use bevy_gantz::head;
use bevy_log as log;
use gantz_ca as ca;
use gantz_egui::HeadDataMut;
use std::collections::HashMap;

/// Bundled query data for open heads. Core data plus views.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct OpenHeadViews {
    pub core: head::OpenHeadData,
    pub view: &'static mut GraphView,
    pub instances: &'static mut HeadNodeInstances,
}

/// The [`gantz_egui::HeadAccess`] implementation for Bevy ECS.
///
/// Wraps the Bevy queries and resources so the gantz_egui widget can access
/// head data without knowing about Bevy's ECS.
pub struct HeadAccess<'q, 'w, 's> {
    /// Heads in tab order, pre-collected.
    heads: Vec<ca::Head>,
    /// Map from head to entity for lookup.
    head_to_entity: HashMap<ca::Head, Entity>,
    /// Query for accessing head data and views mutably.
    query: &'q mut Query<'w, 's, OpenHeadViews, With<head::OpenHead>>,
    /// The VMs keyed by entity.
    vms: &'q mut head::HeadVms,
}

impl<'q, 'w, 's> HeadAccess<'q, 'w, 's> {
    pub fn new(
        tab_order: &head::HeadTabOrder,
        query: &'q mut Query<'w, 's, OpenHeadViews, With<head::OpenHead>>,
        vms: &'q mut head::HeadVms,
    ) -> Self {
        let mut heads = Vec::new();
        let mut head_to_entity = HashMap::new();

        for &entity in tab_order.iter() {
            match query.get(entity) {
                Ok(data) => {
                    let head: ca::Head = (**data.core.head_ref).clone();
                    heads.push(head.clone());
                    head_to_entity.insert(head, entity);
                }
                // A tab-order entity outside the query means a spawn path
                // missed one of the per-head GUI components. Dropping it here
                // would hide the head from the UI while its VM keeps running,
                // so log it loudly instead.
                Err(e) => {
                    log::error!("open head {entity} missing from the views query: {e}");
                }
            }
        }

        Self {
            heads,
            head_to_entity,
            query,
            vms,
        }
    }

    /// Iterate over all heads mutably, for post-GUI updates.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = OpenHeadViewsItem<'_, '_>> + '_ {
        self.query.iter_mut()
    }
}

impl gantz_egui::HeadAccess for HeadAccess<'_, '_, '_> {
    fn heads(&self) -> &[ca::Head] {
        &self.heads
    }

    fn with_head_mut<R>(
        &mut self,
        head: &ca::Head,
        f: impl FnOnce(HeadDataMut<'_>) -> R,
    ) -> Option<R> {
        let entity = *self.head_to_entity.get(head)?;
        let mut data = self.query.get_mut(entity).ok()?;
        let vm = self.vms.get_mut(&entity)?;
        Some(f(HeadDataMut {
            graph: &mut *data.core.working_graph,
            view: &mut *data.view,
            vm,
            instances: &mut data.instances.0,
        }))
    }

    fn module(&self, head: &ca::Head) -> Option<&gantz_core::vm::Compiled> {
        let entity = *self.head_to_entity.get(head)?;
        let data = self.query.get(entity).ok()?;
        data.core.module.compiled.as_ref()
    }

    fn compile_error(&self, head: &ca::Head) -> Option<&str> {
        let entity = *self.head_to_entity.get(head)?;
        let data = self.query.get(entity).ok()?;
        data.core.module.error.as_deref()
    }

    fn diagnostics(&self, head: &ca::Head) -> &[gantz_core::Diagnostic] {
        let Some(&entity) = self.head_to_entity.get(head) else {
            return &[];
        };
        let Ok(data) = self.query.get(entity) else {
            return &[];
        };
        &data.core.diagnostics.0
    }
}
