//! The golden suites: on every fixture ledger of the repository (`zhang_testkit::fixtures`), the API answers what the
//! engine or the store says (`report`, `journals`, `accounts`), and the ids of transactions and assertions stay what
//! the golden file holds (`transaction_ids`).

mod accounts;
mod journals;
mod report;
mod transaction_ids;
