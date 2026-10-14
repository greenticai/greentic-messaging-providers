//! The state-store capability the pack is allowed to use.
//!
//! Mirrors `greentic:state/state-store@1.1.0`: blob read/write/delete plus the
//! atomic `write-if-absent`. There is deliberately no compare-and-swap and no
//! list, so everything the log does is built from those four calls.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The host cannot perform the call (for example `write-if-absent` on a
    /// backend without atomic create). The log refuses to run rather than
    /// risk two writers claiming one cursor.
    Unsupported,
    /// Any other host failure. The text is host-supplied and must not be
    /// shown to API clients.
    Failed(String),
}

pub trait StateStore {
    /// `Ok(None)` when the key is absent (or empty).
    fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn write(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError>;
    fn delete(&mut self, key: &str) -> Result<(), StoreError>;
    /// Atomically create `key`. `Ok(true)` when this call created it,
    /// `Ok(false)` when a value already existed and was left untouched.
    fn write_if_absent(&mut self, key: &str, value: &[u8]) -> Result<bool, StoreError>;
}

#[cfg(test)]
pub mod mem {
    use super::*;
    use std::collections::BTreeMap;

    /// In-memory store. `atomic_create = false` models a backend that answers
    /// `unsupported` to `write-if-absent`.
    #[derive(Default)]
    pub struct MemStore {
        pub map: BTreeMap<String, Vec<u8>>,
        pub no_atomic_create: bool,
    }

    impl StateStore for MemStore {
        fn read(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.map.get(key).cloned())
        }
        fn write(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError> {
            self.map.insert(key.to_string(), value.to_vec());
            Ok(())
        }
        fn delete(&mut self, key: &str) -> Result<(), StoreError> {
            self.map.remove(key);
            Ok(())
        }
        fn write_if_absent(&mut self, key: &str, value: &[u8]) -> Result<bool, StoreError> {
            if self.no_atomic_create {
                return Err(StoreError::Unsupported);
            }
            if self.map.contains_key(key) {
                return Ok(false);
            }
            self.map.insert(key.to_string(), value.to_vec());
            Ok(true)
        }
    }
}
