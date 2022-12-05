use crate::server::model::mutation::MutationRoot;
use crate::server::model::query::QueryRoot;
use async_graphql::{EmptySubscription, Schema};
pub mod mutation;
pub mod query;
pub mod utils;
pub type LedgerSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;
