//! View state resources.

use bevy_ecs::prelude::*;
use gantz_ca as ca;
use std::collections::HashMap;

/// Views (layout + camera) for all known commits.
#[derive(Resource, Default)]
pub struct Views(pub HashMap<ca::CommitAddr, gantz_egui::GraphViews>);

impl std::ops::Deref for Views {
    type Target = HashMap<ca::CommitAddr, gantz_egui::GraphViews>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Views {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
