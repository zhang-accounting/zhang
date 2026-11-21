//! Data that a reader of a loaded ledger computes from it once and keeps with it.

use std::any::Any;
use std::fmt;
use std::sync::OnceLock;

/// What a reader keeps with a loaded ledger ([`Derived`]): data it computed from the ledger once, and which part of it
/// depends on the ledger's errors.
pub trait DerivedData: Any + Send + Sync {
    /// The ledger's errors changed: drop what was computed from them, keep the rest. A load places its errors last,
    /// after its analyzers ran, and their queries may have computed data from the errors before they were placed.
    ///
    /// Only what this drops is computed again. The query engine keeps the snapshots of its native providers here too,
    /// and keeps one an analysis's query built: a native provider must therefore not read the ledger's errors.
    fn errors_changed(&mut self);
}

/// What [`Derived`] keeps: the data, and how to read it as its own type.
trait Slot: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn errors_changed(&mut self);
}

impl<T: DerivedData> Slot for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn errors_changed(&mut self) {
        DerivedData::errors_changed(self)
    }
}

/// Data a reader derives from a loaded ledger on first use and keeps with the ledger, such as
/// the query engine's booked postings. The query engine depends on zhang-core, so the slot does
/// not name its type.
///
/// A loaded ledger does not change: [`Ledger::reload`](crate::ledger::Ledger::reload) replaces
/// it with a new one, whose slot starts empty. The derived data therefore never outlives the
/// ledger it was computed from, and a reload invalidates it. Within a load, the analyzers' queries may compute data
/// before the errors are placed: the load then drops what depends on them ([`Derived::errors_changed`]).
#[derive(Default)]
pub struct Derived {
    slot: OnceLock<Box<dyn Slot>>,
}

impl Derived {
    /// The data, computed with `init` on first use; concurrent first uses wait for one `init`.
    ///
    /// The slot holds one type of data: `None` when data of another type was stored first.
    pub fn get_or_init<T: DerivedData>(&self, init: impl FnOnce() -> T) -> Option<&T> {
        self.slot.get_or_init(|| Box::new(init())).as_any().downcast_ref()
    }

    /// Whether the data was computed.
    pub fn is_initialized(&self) -> bool {
        self.slot.get().is_some()
    }

    /// The ledger's errors changed: the data drops what it computed from them ([`DerivedData::errors_changed`]).
    pub fn errors_changed(&mut self) {
        if let Some(slot) = self.slot.get_mut() {
            slot.errors_changed();
        }
    }
}

impl fmt::Debug for Derived {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Derived").field("initialized", &self.is_initialized()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Derived, DerivedData};

    #[derive(Debug, PartialEq)]
    struct Counted {
        value: u32,
        from_errors: Option<u32>,
    }

    impl DerivedData for Counted {
        fn errors_changed(&mut self) {
            self.from_errors = None;
        }
    }

    impl DerivedData for &'static str {
        fn errors_changed(&mut self) {}
    }

    #[test]
    fn computes_once_and_keeps_one_type() {
        let mut derived = Derived::default();
        assert!(!derived.is_initialized());
        let counted = |value| Counted {
            value,
            from_errors: Some(value),
        };
        assert_eq!(derived.get_or_init(|| counted(1)), Some(&counted(1)));
        assert_eq!(derived.get_or_init(|| counted(2)), Some(&counted(1)));
        assert!(derived.is_initialized());
        assert_eq!(derived.get_or_init(|| "other"), None);
        // what depends on the errors goes, the rest stays
        derived.errors_changed();
        assert_eq!(derived.get_or_init(|| counted(3)), Some(&Counted { value: 1, from_errors: None }));
    }
}
