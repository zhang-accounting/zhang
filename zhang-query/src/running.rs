//! The running totals behind the `balance` column ([`RunningState`]): the running inventory
//! of the rows added so far.

use crate::compiler::Running;
use crate::table::{self, Row};
use crate::value::Inventory;

/// The running totals of one pass over the filtered rows.
pub(crate) struct RunningState {
    balance: Inventory,
}

impl RunningState {
    /// A state keeping the `totals`.
    pub fn new(_totals: &[Running]) -> Self {
        RunningState { balance: Inventory::new() }
    }

    /// Add a row that passed the filter.
    pub fn add(&mut self, row: &Row<'_>) {
        self.balance.add_owned_position(table::position(row));
    }

    /// The value of the running `total`.
    pub fn value(&self, total: Running) -> Inventory {
        match total {
            Running::Balance => self.balance.clone(),
        }
    }
}
