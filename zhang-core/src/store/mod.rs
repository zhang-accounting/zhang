use std::collections::{BTreeMap, HashMap, HashSet};

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use indexmap::IndexMap;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Flag, Meta, SpanInfo};

use crate::constants::BALANCE_CHECK_PAYEE;
use crate::domains::schemas::{AccountDomain, CommodityDomain, ErrorDomain, MetaDomain, PriceDomain, QueryDomain};

#[derive(Default, serde::Serialize)]
pub struct Store {
    pub options: HashMap<String, String>,
    pub accounts: HashMap<String, AccountDomain>,
    pub commodities: IndexMap<String, CommodityDomain>,
    pub transactions: HashMap<Uuid, TransactionDomain>,
    pub postings: Vec<PostingDomain>,

    /// the `balance` assertions, in ledger order. They are not transactions and have no postings:
    /// an assertion changes no balance
    pub balance_assertions: Vec<BalanceAssertionDomain>,

    pub prices: Vec<PriceDomain>,

    pub budgets: HashMap<String, BudgetDomain>,

    // by account
    pub commodity_lots: HashMap<String, Vec<CommodityLotRecord>>,

    pub documents: Vec<DocumentDomain>,

    /// saved queries from `query` directives, in ledger order (by date, then source order)
    pub queries: Vec<QueryDomain>,

    pub metas: Vec<MetaDomain>,

    pub errors: Vec<ErrorDomain>,
}

#[derive(Clone, serde::Serialize, Debug)]
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
    pub fn match_keywords(&self, keyword: Option<&String>, tags: &Option<HashSet<String>>, links: &Option<HashSet<String>>) -> bool {
        let keyword = keyword.map(|it| it.to_lowercase());
        let tag_matched = if let Some(tag_candidates) = tags.as_ref() {
            // if one of the tags matched, return the transaction
            self.tags.iter().any(|trx_tag| tag_candidates.contains(trx_tag))
        } else {
            // if tags are not specified, all transactions are matched
            true
        };
        let link_matched = if let Some(link_candidates) = links.as_ref() {
            // if one of the links matched, return the transaction
            self.links.iter().any(|trx_link| link_candidates.contains(trx_link))
        } else {
            // if tags are not specified, all transactions are matched
            true
        };
        let keyword_matched = ({
            if let Some(keyword) = keyword.as_ref() {
                let is_payee_matched = self.payee.as_ref().map(|it| it.to_lowercase().contains(keyword)).unwrap_or(false);
                is_payee_matched
            } else {
                true
            }
        }) || ({
            if let Some(keyword) = keyword.as_ref() {
                let is_narration_matched = self.narration.as_ref().map(|it| it.to_lowercase().contains(keyword)).unwrap_or(false);
                is_narration_matched
            } else {
                true
            }
        }) || ({
            if let Some(keyword) = keyword.as_ref() {
                let is_any_tags_matched = self.tags.iter().any(|it| it.to_lowercase().contains(keyword));
                is_any_tags_matched
            } else {
                true
            }
        }) || ({
            if let Some(keyword) = keyword.as_ref() {
                let is_any_links_matched = self.links.iter().any(|it| it.to_lowercase().contains(keyword));
                is_any_links_matched
            } else {
                true
            }
        }) || ({
            if let Some(keyword) = keyword.as_ref() {
                let is_any_posting_account_matched = self.postings.iter().any(|posting| posting.account.name().to_lowercase().contains(keyword));
                is_any_posting_account_matched
            } else {
                true
            }
        });
        tag_matched && link_matched && keyword_matched
    }
}

/// A `balance` assertion as the load checked it: the asserted amount next to the account's balance
/// where the assertion stands.
///
/// It records a check, it is not a transaction: it has no postings, so nothing that sums postings
/// books it, and no balance depends on it. A failing assertion is reported as an
/// `AccountBalanceCheckError` and changes no number; `balance ... with pad` (a `P` transaction)
/// is how a balance is corrected on purpose.
#[derive(Clone, serde::Serialize, Debug)]
pub struct BalanceAssertionDomain {
    /// derived from the span of the `balance` directive, like a transaction id
    pub id: Uuid,
    /// its place in the journal: assertions and transactions share one sequence
    pub sequence: i32,
    pub datetime: DateTime<Tz>,
    pub account: Account,
    /// the asserted amount
    pub amount: Amount,
    /// the explicit tolerance (`~`); `None` asserts the exact amount
    pub tolerance: Option<BigDecimal>,
    /// the account's balance in the asserted currency where the assertion stands: the sum of the
    /// postings of the account and all its sub-accounts before it, as in beancount
    pub balance: Amount,
    /// whether `balance` is within `tolerance` of `amount`
    pub passed: bool,
    pub span: SpanInfo,
}

impl BalanceAssertionDomain {
    /// whether a journal search matches the assertion, the way [`TransactionDomain::match_keywords`] matches a
    /// transaction. The journal lists an assertion with the payee `Balance Check` and its account as the narration
    /// and the only account; it has no tags or links, so a search by tag or link never matches it
    pub fn match_keywords(&self, keyword: Option<&String>, tags: &Option<HashSet<String>>, links: &Option<HashSet<String>>) -> bool {
        if tags.is_some() || links.is_some() {
            return false;
        }
        let Some(keyword) = keyword.map(|it| it.to_lowercase()) else {
            return true;
        };
        BALANCE_CHECK_PAYEE.to_lowercase().contains(&keyword) || self.account.name().to_lowercase().contains(&keyword)
    }
}

#[derive(Clone, serde::Serialize, Debug)]
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
    /// metadata of the posting, sorted by key (the values of a repeated key in ledger
    /// order). The transaction's own metadata is in [`Store::metas`].
    pub metas: Vec<PostingMetaDomain>,
}

/// One metadata entry of a posting, its value as plain text like [`MetaDomain`]'s.
#[derive(Clone, serde::Serialize, Debug, PartialEq, Eq)]
pub struct PostingMetaDomain {
    pub key: String,
    pub value: String,
}

impl PostingMetaDomain {
    /// The entries of `meta`, sorted by key; the values of a repeated key keep their order.
    pub fn of(meta: Meta) -> Vec<PostingMetaDomain> {
        let mut metas = meta
            .get_flatten()
            .into_iter()
            .map(|(key, value)| PostingMetaDomain {
                key,
                value: value.to_plain_string(),
            })
            .collect::<Vec<_>>();
        metas.sort_by(|a, b| a.key.cmp(&b.key));
        metas
    }
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

#[derive(Default, Clone, Debug, serde::Serialize, PartialEq)]
pub struct CommodityLotRecord {
    pub commodity: String,
    pub amount: BigDecimal,

    pub cost: Option<Amount>,

    // acquisition date
    pub acquisition_date: Option<NaiveDate>,
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
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum BudgetEventType {
    AddAssignedAmount,
    Transfer,
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use uuid::uuid;
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
