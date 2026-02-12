//! App-specific storage utilities.
//!
//! Provides the [`Pkv`] newtype implementing [`bevy_gantz::storage::Load`] and
//! [`bevy_gantz::storage::Save`] for [`bevy_pkv::PkvStore`].

use bevy::prelude::Resource;
use bevy_gantz::storage::{Load, Save};
use bevy_pkv::PkvStore;

/// A [`Resource`] wrapping [`PkvStore`] that implements [`Load`] and [`Save`].
#[derive(Resource)]
pub struct Pkv(pub PkvStore);

impl Load for Pkv {
    type Err = bevy_pkv::GetError;
    fn get_string(&self, key: &str) -> Result<Option<String>, Self::Err> {
        match self.0.get::<String>(key) {
            Ok(v) => Ok(Some(v)),
            Err(bevy_pkv::GetError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Save for Pkv {
    type Err = bevy_pkv::SetError;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        self.0.set_string(key, value)
    }
}
