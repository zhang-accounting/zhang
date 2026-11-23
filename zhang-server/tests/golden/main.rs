//! The golden suites: on every fixture ledger of the repository (`zhang_testkit::fixtures`), the API answers what the
//! engine or the store says (`report`, `journals`, `accounts`), the budget pages' built-in queries answer what an
//! independent computation of the budgets gives (`budget_commodity`, with `budget_reference`), and the ids of
//! transactions and assertions stay what the golden file holds (`transaction_ids`). `budget_contract` freezes what a
//! client of the budget system sees on hand-written ledgers, the contract of the budget plugin (#698), which the bundled
//! script `zhang:budget`, the only implementation of budgets a server runs, answers, and the native budgets of RFC #698's
//! test switch too; `budget_parity` and `budget_fuzz` compare the script with the native budgets on generated ledgers.

mod accounts;
mod budget_commodity;
mod budget_contract;
mod budget_fuzz;
mod budget_parity;
mod budget_reference;
mod journals;
mod report;
mod transaction_ids;
