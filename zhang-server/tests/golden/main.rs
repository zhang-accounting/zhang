//! The golden suites: on every fixture ledger of the repository (`zhang_testkit::fixtures`), the API answers what the
//! engine or the store says (`report`, `journals`, `accounts`), the budget pages' built-in queries answer what an
//! independent computation of the budgets gives (`budget_commodity`, with `budget_reference`), and the ids of
//! transactions and assertions stay what the golden file holds (`transaction_ids`).

mod accounts;
mod budget_commodity;
mod budget_reference;
mod journals;
mod report;
mod transaction_ids;
