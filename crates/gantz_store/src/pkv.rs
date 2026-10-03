use crate::{Load, Save};
use bevy_pkv::{GetError, PkvStore, SetError};

impl Load for PkvStore {
    type Err = GetError;
    fn get_string(&self, key: &str) -> Result<Option<String>, Self::Err> {
        match self.get::<String>(key) {
            Ok(v) => Ok(Some(v)),
            Err(GetError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Save for PkvStore {
    type Err = SetError;
    fn set_string(&mut self, key: &str, value: &str) -> Result<(), Self::Err> {
        PkvStore::set_string(self, key, value)
    }
}
