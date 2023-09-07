use crate::domains::schemas::{AccountDomain, CommodityDomain, ErrorDomain, ErrorType, MetaDomain, PriceDomain, TransactionInfoDomain};
use bigdecimal::BigDecimal;
use chrono::DateTime;
use chrono_tz::Tz;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Flag, SpanInfo, Transaction};

pub struct TransactionHeaderDomain {
    pub id: Uuid,
    pub datetime: DateTime<Tz>,
    pub flag: Flag,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub span: SpanInfo,
    pub tags: Vec<String>,
    pub links: Vec<String>,
}

pub struct PostingDomain {
    pub id: Uuid,
    pub trx_id: Uuid,
    pub trx_datetime: DateTime<Tz>,
    pub account: Account,
    pub unit: Option<Amount>,
    pub cost: Option<Amount>,
    pub inferred_amount: Amount,
    pub previous_amount: Amount,
    pub after_amount: Amount,
}

pub enum DocumentType {
    Trx(Uuid),
    Account(Account),
}

pub struct DocumentDomain {
    pub datetime: DateTime<Tz>,
    pub document_type: DocumentType,
    pub filename: Option<String>,
    pub path: String,
}

pub struct Store {
    pub(crate) options: HashMap<String, String>,
    pub(crate) accounts: HashMap<Account, AccountDomain>,
    pub(crate) commodities: HashMap<String, CommodityDomain>,
    pub(crate) transactions: HashMap<Uuid, TransactionHeaderDomain>,
    pub(crate) postings: Vec<PostingDomain>,

    pub(crate) prices: Vec<PriceDomain>,

    // by account
    pub(crate) commodity_lots: HashMap<Account, Vec<CommodityLotRecord>>,

    pub(crate) documents: Vec<DocumentDomain>,

    pub(crate) metas: Vec<MetaDomain>,

    pub(crate) errors: Vec<ErrorDomain>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            options: HashMap::default(),
            accounts: Default::default(),
            commodities: Default::default(),
            transactions: Default::default(),
            postings: vec![],
            prices: vec![],
            commodity_lots: Default::default(),
            documents: vec![],
            metas: vec![],
            errors: vec![],
        }
    }
}

#[derive(Default, Clone)]
pub struct CommodityLotRecord {
    pub commodity: String,
    pub datetime: Option<DateTime<Tz>>,
    pub amount: BigDecimal,
    pub price: Option<Amount>,
}
