use std::collections::{BTreeMap, HashMap};

use bigdecimal::BigDecimal;
use chrono::DateTime;
use chrono_tz::Tz;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Flag, SpanInfo};

use crate::domains::schemas::{AccountDomain, CommodityDomain, ErrorDomain, MetaDomain, PriceDomain};

#[derive(Default, serde::Serialize)]
pub struct Store {
    pub options: HashMap<String, String>,
    pub accounts: HashMap<String, AccountDomain>,
    pub commodities: HashMap<String, CommodityDomain>,
    pub transactions: HashMap<Uuid, TransactionDomain>,
    pub postings: Vec<PostingDomain>,

    pub prices: Vec<PriceDomain>,

    pub budgets: HashMap<String, BudgetDomain>,

    // by account
    pub commodity_lots: HashMap<String, Vec<CommodityLotRecord>>,

    pub documents: Vec<DocumentDomain>,

    pub metas: Vec<MetaDomain>,

    pub errors: Vec<ErrorDomain>,
}

#[derive(Clone, serde::Serialize)]
pub struct TransactionDomain {
    pub id: Uuid,
    pub sequence: i32,
    pub datetime: DateTime<Tz>,
    pub flag: Flag,
    pub payee: Option<String>,
    pub narration: Option<String>,
    pub span: SpanInfo,
    pub tags: Vec<String>,
    pub links: Vec<String>,
    pub postings: Vec<PostingDomain>,
}

impl TransactionDomain {
    pub fn contains_keyword(&self, keyword: &str) -> bool {
        let keyword = keyword.to_lowercase();
        ({
            let is_payee_matched = self.payee.as_ref().map(|it| it.to_lowercase().contains(&keyword)).unwrap_or(false);
            is_payee_matched
        }) || ({
            let is_narration_matched = self.narration.as_ref().map(|it| it.to_lowercase().contains(&keyword)).unwrap_or(false);
            is_narration_matched
        }) || ({
            let is_any_tags_matched = self.tags.iter().any(|it| it.to_lowercase().contains(&keyword));
            is_any_tags_matched
        }) || ({
            let is_any_links_matched = self.links.iter().any(|it| it.to_lowercase().contains(&keyword));
            is_any_links_matched
        }) || ({
            let is_any_posting_account_matched = self.postings.iter().any(|posting| posting.account.name().to_lowercase().contains(&keyword));
            is_any_posting_account_matched
        })
    }
}

#[derive(Clone, serde::Serialize)]
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

#[derive(Clone, serde::Serialize)]
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

#[derive(Clone, serde::Serialize)]
pub struct DocumentDomain {
    pub datetime: DateTime<Tz>,
    pub document_type: DocumentType,
    pub filename: Option<String>,
    pub path: String,
}

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct CommodityLotRecord {
    pub commodity: String,
    pub datetime: Option<DateTime<Tz>>,
    pub amount: BigDecimal,
    pub price: Option<Amount>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetDomain {
    pub name: String,
    pub alias: Option<String>,
    pub category: Option<String>,
    pub closed: bool,
    pub detail: BTreeMap<u32, BudgetIntervalDetail>,
    pub commodity: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetIntervalDetail {
    /// year and month pair, calculated as `year*100+month`, E.G. `202312`
    pub date: u32,
    pub assigned_amount: Amount,
    // todo: budget event for addition, transfer and close
    pub events: Vec<BudgetEvent>,
    pub activity_amount: Amount,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BudgetEvent {
    pub datetime: DateTime<Tz>,
    pub timestamp: i64,
    pub amount: Amount,
    pub event_type: BudgetEventType,
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum BudgetEventType {
    AddAssignedAmount,
    Transfer,
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use uuid::{uuid, Uuid};
    use zhang_ast::Account;

    use crate::store::DocumentType;

    #[test]
    fn should_match_document_type() {
        let document_type = DocumentType::Trx(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8"));
        assert!(!document_type.match_account("any"));

        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());

        assert!(account_type.match_account("Assets:A"));
        assert!(!account_type.match_account("Assets:A:B"));
        assert!(!account_type.match_account("Assets:C"));
    }

    #[test]
    fn should_return_account() {
        let document_type = DocumentType::Trx(uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8"));
        assert_eq!(None, document_type.as_account());

        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());
        assert_eq!(account_type.as_account(), Some("Assets:A".to_owned()));
    }

    #[test]
    fn should_return_trx() {
        let uuid = uuid!("67e55044-10b1-426f-9247-bb680e5fe0c8");
        let document_type = DocumentType::Trx(uuid);
        assert_eq!(Some(uuid.to_string()), document_type.as_trx());

        let account_type = DocumentType::Account(Account::from_str("Assets:A").unwrap());
        assert_eq!(account_type.as_trx(), None);
    }
}
