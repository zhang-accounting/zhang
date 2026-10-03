//! Data that a reader of a loaded ledger computes from it once and keeps with it.

use std::any::Any;
use std::fmt;
use std::sync::OnceLock;

/// Data a reader derives from a loaded ledger on first use and keeps with the ledger, such as
/// the query engine's booked postings. The query engine depends on zhang-core, so the slot does
/// not name its type.
///
/// A loaded ledger does not change: [`Ledger::reload`](crate::ledger::Ledger::reload) replaces
/// it with a new one, whose slot starts empty. The derived data therefore never outlives the
/// ledger it was computed from, and a reload invalidates it.
#[derive(Default)]
pub struct Derived {
    slot: OnceLock<Box<dyn Any + Send + Sync>>,
}

impl Derived {
    /// The data, computed with `init` on first use; concurrent first uses wait for one `init`.
    ///
    /// The slot holds one type of data: `None` when data of another type was stored first.
    pub fn get_or_init<T: Any + Send + Sync>(&self, init: impl FnOnce() -> T) -> Option<&T> {
        self.slot.get_or_init(|| Box::new(init())).downcast_ref()
    }

    /// Whether the data was computed.
    pub fn is_initialized(&self) -> bool {
        self.slot.get().is_some()
    }
}

impl fmt::Debug for Derived {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Derived").field("initialized", &self.is_initialized()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Derived;

    #[test]
    fn computes_once_and_keeps_one_type() {
        let derived = Derived::default();
        assert!(!derived.is_initialized());
        assert_eq!(derived.get_or_init(|| 1_u32), Some(&1));
        assert_eq!(derived.get_or_init(|| 2_u32), Some(&1));
        assert!(derived.is_initialized());
        assert_eq!(derived.get_or_init(|| "other"), None);
    }
}
