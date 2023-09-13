use std::collections::HashMap;

use bigdecimal::BigDecimal;
use chrono::DateTime;
use chrono_tz::Tz;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Flag, SpanInfo};

use crate::domains::schemas::{AccountDomain, CommodityDomain, ErrorDomain, MetaDomain, PriceDomain};

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Default)]
pub struct Store {
    pub options: HashMap<String, String>,
    pub accounts: HashMap<Account, AccountDomain>,
    pub commodities: HashMap<String, CommodityDomain>,
    pub transactions: HashMap<Uuid, TransactionHeaderDomain>,
    pub postings: Vec<PostingDomain>,

    pub prices: Vec<PriceDomain>,

    // by account
    pub commodity_lots: HashMap<Account, Vec<CommodityLotRecord>>,

    pub documents: Vec<DocumentDomain>,

    pub metas: Vec<MetaDomain>,

    pub errors: Vec<ErrorDomain>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Clone)]
pub struct TransactionHeaderDomain {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: DateTime<Tz>,
    pub flag: Flag,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub span: SpanInfo,
    pub tags: Vec<String>,
    pub links: Vec<String>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Clone)]
pub struct PostingDomain {
    pub id: Uuid,
    pub trx_id: Uuid,
    pub trx_sequence: i32,
    pub trx_datetime: DateTime<Tz>,
    pub account: Account,
    pub unit: Option<Amount>,
    pub cost: Option<Amount>,
    pub inferred_amount: Amount,
    pub previous_amount: Amount,
    pub after_amount: Amount,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Clone)]
pub enum DocumentType {
    Trx(Uuid),
    Account(Account),
}

impl DocumentType {
    pub fn match_account(&self, account_name: &str) -> bool {
        match self {
            DocumentType::Trx(_) => false,
            DocumentType::Account(acc) => acc.name().eq(account_name),
        }
    }
    pub fn as_account(&self) -> Option<String> {
        match self {
            DocumentType::Trx(_) => None,
            DocumentType::Account(account) => Some(account.name().to_owned()),
        }
    }
    pub fn as_trx(&self) -> Option<String> {
        match self {
            DocumentType::Trx(id) => Some(id.to_string()),
            DocumentType::Account(_) => None,
        }
    }
}


#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Clone)]
pub struct DocumentDomain {
    pub datetime: DateTime<Tz>,
    pub document_type: DocumentType,
    pub filename: Option<String>,
    pub path: String,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[derive(Default, Clone, Debug)]
pub struct CommodityLotRecord {
    pub commodity: String,
    pub datetime: Option<DateTime<Tz>>,
    pub amount: BigDecimal,
    pub price: Option<Amount>,
}
