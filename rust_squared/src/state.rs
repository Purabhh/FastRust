use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::RsqError;

/// Type-keyed map for per-request dependency caching.
/// Replaces the anymap2 dependency with a minimal internal implementation.
pub(crate) struct TypeMap {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl TypeMap {
    pub(crate) fn new() -> Self {
        Self { map: HashMap::new() }
    }

    pub(crate) fn insert<T: Any + Send + Sync>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    pub(crate) fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }
}

impl Default for TypeMap {
    fn default() -> Self { Self::new() }
}

#[derive(Clone, Default)]
pub struct AppState {
    entries: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert<T>(&mut self, value: T) -> Result<(), RsqError>
    where
        T: Clone + Send + Sync + 'static,
    {
        let map = Arc::make_mut(&mut self.entries);
        let type_id = TypeId::of::<T>();
        if map.contains_key(&type_id) {
            return Err(RsqError::bad_request(format!(
                "state for `{}` already registered",
                std::any::type_name::<T>()
            )));
        }
        map.insert(type_id, Arc::new(value));
        Ok(())
    }

    pub fn get<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.entries
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
    }
}
