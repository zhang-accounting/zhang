use async_graphql::{EmptyMutation, EmptySubscription, Schema};
use crate::server::model::mutation::MutationRoot;
use crate::server::model::query::QueryRoot;

pub mod query;
pub mod mutation;

pub type LedgerSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;
