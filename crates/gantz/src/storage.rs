//! App-specific storage utilities.
//!
//! Provides the [`Pkv`] newtype implementing [`bevy_gantz::storage::Load`] and
//! [`bevy_gantz::storage::Save`] for [`bevy_pkv::PkvStore`].

use bevy::prelude::Resource;
use bevy_gantz::storage::{Load, Save};
use bevy_pkv::PkvStore;
use std::sync::{Arc, Mutex};

/// A [`Resource`] wrapping a shared [`PkvStore`] that implements [`Load`] and
/// [`Save`].
///
/// `Arc<Mutex<_>>` lets a background persistence worker share the single store
/// handle with the main thread. redb permits only one handle per database
/// file.
///
/// Despite the `Mutex`, there is no lock contention during the render loop.
/// The main thread only reads the store, during startup and the first-frame
/// egui-memory load. A debounced persist needs prior input, so none can fire
/// before then. From then on the worker is the sole accessor. The persist
/// systems hand their writes to it over a channel rather than lock the store.
/// So the worker's fsync'd writes never block a frame.
#[derive(Resource, Clone)]
pub struct Pkv(pub Arc<Mutex<PkvStore>>);

impl Pkv {
    /// Wrap a store for sharing between the main thread and the worker.
    pub fn new(store: PkvStore) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }
}

impl Load for Pkv {
    type Err = bevy_pkv::GetError;
    fn get_string(&self, key: &str) -> Result<Option<String>, Self::Err> {
        match self.0.lock().unwrap().get::<String>(key) {
            Ok(v) => Ok(Some(v)),
            Err(bevy_pkv::GetError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Save for Pkv {
    type Err = bevy_pkv::SetError;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        self.0.lock().unwrap().set_string(key, value)
    }
}
