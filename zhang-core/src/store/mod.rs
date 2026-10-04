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

use crate::domains::schemas::{AccountDomain, CommodityDomain, ErrorDomain, MetaDomain, PriceDomain, QueryDomain};

#[derive(Default, serde::Serialize)]
pub struct Store {
    pub options: HashMap<String, String>,
    pub accounts: HashMap<String, AccountDomain>,
    pub commodities: IndexMap<String, CommodityDomain>,
    pub transactions: HashMap<Uuid, TransactionDomain>,
    /// in insertion order. The load appends with `Store::push_posting`, which keeps the index of
    /// `Store::last_posting_at` in step; nothing removes or reorders a posting
    pub postings: Vec<PostingDomain>,
    /// per account and commodity of `after_amount`, the positions in [`Store::postings`] of its postings, sorted by
    /// `(trx_datetime, position)`. A posting pushed to `postings` directly, not with [`Store::push_posting`], is not
    /// in it
    #[serde(skip)]
    posting_index: HashMap<String, HashMap<String, Vec<usize>>>,

    /// the `balance` assertions, in ledger order. They are not transactions and have no postings:
    /// an assertion changes no balance
    pub balance_assertions: Vec<BalanceAssertionDomain>,
    /// the ids of [`Store::balance_assertions`], which an id given to a transaction or an assertion avoids
    #[serde(skip)]
    pub(crate) balance_assertion_ids: HashSet<Uuid>,

    pub prices: Vec<PriceDomain>,

    pub budgets: HashMap<String, BudgetDomain>,

    // by account
    pub commodity_lots: HashMap<String, Vec<CommodityLotRecord>>,

    pub documents: Vec<DocumentDomain>,

    /// saved queries from `query` directives, in ledger order (by date, then source order)
    pub queries: Vec<QueryDomain>,

    /// in insertion order. `Operations::insert_meta` appends with [`Store::push_meta`], which keeps the index of
    /// [`Store::metas_of`] in step; a meta pushed to `metas` directly is found by a scan of the entries after the
    /// indexed ones. Nothing removes, reorders or clears a meta
    pub metas: Vec<MetaDomain>,
    /// per meta type and type identifier, the positions in [`Store::metas`] of the first `indexed_metas` entries,
    /// in store order
    #[serde(skip)]
    meta_index: HashMap<String, HashMap<String, Vec<usize>>>,
    /// how many leading entries of [`Store::metas`] are in `meta_index`
    #[serde(skip)]
    indexed_metas: usize,

    pub errors: Vec<ErrorDomain>,
}

impl Store {
    /// append a posting to [`Store::postings`] and index it. Every indexed position is lower than the new one, so the
    /// new position goes after those dated on or before the posting and before those dated after it, which keeps
    /// the positions sorted by `(trx_datetime, position)` when postings come out of datetime order
    pub(crate) fn push_posting(&mut self, posting: PostingDomain) {
        let position = self.postings.len();
        let datetime = posting.trx_datetime;
        let positions = self
            .posting_index
            .entry(posting.account.name().to_owned())
            .or_default()
            .entry(posting.after_amount.commodity.clone())
            .or_default();
        self.postings.push(posting);
        let postings = &self.postings;
        positions.insert(positions.partition_point(|&it| postings[it].trx_datetime <= datetime), position);
    }

    /// of the postings of `account` whose `after_amount` is in `commodity`, the latest dated on or before `datetime`,
    /// and of several dated the same, the last inserted: the last of a stable sort by datetime
    pub(crate) fn last_posting_at(&self, account: &str, commodity: &str, datetime: DateTime<Tz>) -> Option<&PostingDomain> {
        let positions = self.posting_index.get(account)?.get(commodity)?;
        let on_or_before = positions.partition_point(|&it| self.postings[it].trx_datetime <= datetime);
        on_or_before.checked_sub(1).map(|it| &self.postings[positions[it]])
    }

    /// append a meta to [`Store::metas`] and index it, with any meta pushed to `metas` directly since the last one
    pub(crate) fn push_meta(&mut self, meta: MetaDomain) {
        self.metas.push(meta);
        self.index_metas();
    }

    /// index the entries of [`Store::metas`] after the indexed ones. Their positions are higher than every indexed
    /// position, so each per-owner vector stays in store order
    fn index_metas(&mut self) {
        for position in self.indexed_metas..self.metas.len() {
            let meta = &self.metas[position];
            self.meta_index
                .entry(meta.meta_type.clone())
                .or_default()
                .entry(meta.type_identifier.clone())
                .or_default()
                .push(position);
        }
        self.indexed_metas = self.metas.len();
    }

    /// the positions in [`Store::metas`] of the metas of `type_identifier` of `meta_type`, in store order: the
    /// indexed ones, then those pushed directly after the last indexing
    fn meta_positions<'a>(&'a self, meta_type: &'a str, type_identifier: &'a str) -> impl Iterator<Item = usize> + 'a {
        let indexed = self
            .meta_index
            .get(meta_type)
            .and_then(|by_identifier| by_identifier.get(type_identifier))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let unindexed = (self.indexed_metas..self.metas.len()).filter(move |&position| {
            let meta = &self.metas[position];
            meta.meta_type == meta_type && meta.type_identifier == type_identifier
        });
        indexed.iter().copied().chain(unindexed)
    }

    /// the metas of `type_identifier` of `meta_type`, in store order: what a scan of [`Store::metas`] filtered by
    /// type and identifier yields
    pub(crate) fn metas_of<'a>(&'a self, meta_type: &'a str, type_identifier: &'a str) -> impl Iterator<Item = &'a MetaDomain> + 'a {
        self.meta_positions(meta_type, type_identifier).map(|position| &self.metas[position])
    }

    /// the position in [`Store::metas`] of the first meta of `type_identifier` of `meta_type` with `key`
    pub(crate) fn meta_position(&self, meta_type: &str, type_identifier: &str, key: &str) -> Option<usize> {
        self.meta_positions(meta_type, type_identifier)
            .find(|&position| self.metas[position].key == key)
    }
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
    /// where the document may be instead, looked at when nothing is at `path`: a `document` of a beancount ledger on
    /// a remote source, whose files are not looked at on load, may be at its path relative to the ledger's root, as
    /// earlier versions wrote it, rather than relative to its file, as beancount reads it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<String>,
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

#[cfg(test)]
mod posting_index_test {
    use bigdecimal::{BigDecimal, Zero};
    use chrono::{DateTime, Duration, TimeZone};
    use chrono_tz::Tz;
    use itertools::Itertools;
    use uuid::Uuid;
    use zhang_ast::amount::Amount;
    use zhang_ast::{Flag, Meta, SpanInfo};

    use crate::domains::Operations;
    use crate::store::Store;

    /// the day balance as found before the postings were indexed: scan every posting, keep those of the account
    /// and commodity dated on or before `datetime`, sort them by datetime (a stable sort) and take the last
    fn scan(store: &Store, account: &str, commodity: &str, datetime: DateTime<Tz>) -> Option<BigDecimal> {
        store
            .postings
            .iter()
            .filter(|posting| posting.account.name() == account)
            .filter(|posting| posting.after_amount.commodity == commodity)
            .filter(|posting| posting.trx_datetime <= datetime)
            .sorted_by_key(|posting| posting.trx_datetime)
            .next_back()
            .map(|posting| posting.after_amount.number.clone())
    }

    /// After every transaction, the indexed lookup finds the posting a scan of every posting finds, at every datetime
    /// of the ledger and between them. The transactions come out of datetime order, many postings share a datetime
    /// (also within one transaction), and they spread over an account, its sub-account and another account, each in
    /// several commodities. Every posting has its own number, so a lookup that finds another posting fails
    #[test]
    fn the_day_balance_lookup_finds_what_a_scan_of_every_posting_finds() {
        let timezone = Tz::Asia__Shanghai;
        let mut operations = Operations {
            timezone,
            store: Default::default(),
        };
        let accounts = ["Assets:Bank", "Assets:Bank:Card", "Expenses:Food"];
        let commodities = ["CNY", "USD", "BTC"];
        // a few datetimes, two of them a second apart, so that many postings share one
        let datetimes = [(1, 1, 0, 0), (1, 1, 0, 1), (1, 2, 12, 0), (2, 1, 0, 0), (3, 1, 0, 0)]
            .map(|(month, day, hour, second)| timezone.with_ymd_and_hms(2024, month, day, hour, 0, second).unwrap());
        // every datetime, an hour before and an hour after it: before all, between and after all of them
        let targets = datetimes
            .iter()
            .flat_map(|datetime| [*datetime - Duration::hours(1), *datetime, *datetime + Duration::hours(1)])
            .collect_vec();

        // xorshift: the same sequence on every run
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut below = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        let mut latest = datetimes[0];
        let mut out_of_order = 0;
        for sequence in 0..120 {
            let id = Uuid::from_u128(sequence as u128 + 1);
            let datetime = datetimes[below(datetimes.len())];
            if datetime < latest {
                out_of_order += 1;
            }
            latest = latest.max(datetime);
            operations
                .insert_transaction(&id, sequence, datetime, Flag::Okay, None, None, vec![], vec![], &SpanInfo::default())
                .unwrap();
            for posting_idx in 0..1 + below(3) {
                let account = accounts[below(accounts.len())];
                let commodity = commodities[below(commodities.len())];
                let zero = Amount::new(BigDecimal::zero(), commodity);
                let after = Amount::new(BigDecimal::from(sequence * 10 + posting_idx as i32), commodity);
                operations
                    .insert_transaction_posting(&id, posting_idx, account, None, None, zero.clone(), zero, after, Meta::default())
                    .unwrap();
            }

            for account in accounts {
                for commodity in commodities.into_iter().chain(["EUR"]) {
                    for target in targets.iter().copied() {
                        let found = operations.account_target_day_balance(account, target, commodity).unwrap();
                        let scanned = scan(&operations.read(), account, commodity, target);
                        assert_eq!(
                            found.map(|it| it.number),
                            scanned,
                            "{account} in {commodity} at {target}, after transaction {sequence}"
                        );
                    }
                }
            }
        }

        let store = operations.read();
        let sharing_a_datetime = store
            .postings
            .iter()
            .map(|posting| (posting.account.name(), &posting.after_amount.commodity, posting.trx_datetime))
            .duplicates()
            .count();
        assert!(out_of_order > 0, "some transactions come out of datetime order");
        assert!(sharing_a_datetime > 0, "some postings of an account and commodity share a datetime");
    }
}

#[cfg(test)]
mod meta_index_test {
    use chrono_tz::Tz;
    use itertools::Itertools;
    use zhang_ast::{Meta, ZhangString};

    use crate::domains::schemas::{MetaDomain, MetaType};
    use crate::domains::Operations;

    /// the fields of a meta, to compare metas and to tell which entry a lookup found
    fn fields(meta: &MetaDomain) -> (String, String, String, String) {
        (meta.meta_type.clone(), meta.type_identifier.clone(), meta.key.clone(), meta.value.clone())
    }

    /// the metas of an owner as `metas` found them before the metas were indexed: scan every meta, keep the owner's,
    /// in store order
    fn scan<'a>(metas: &'a [MetaDomain], type_: &'a MetaType, type_identifier: &'a str) -> impl Iterator<Item = &'a MetaDomain> + 'a {
        metas
            .iter()
            .filter(move |meta| meta.meta_type == type_.as_ref())
            .filter(move |meta| meta.type_identifier == type_identifier)
    }

    /// `insert_meta` of one entry as it was before the metas were indexed: scan every meta for the owner's key and
    /// update the first in place, or append. Tells whether it updated
    fn scan_insert(metas: &mut Vec<MetaDomain>, type_: &MetaType, type_identifier: &str, key: &str, value: &str) -> bool {
        let found = metas
            .iter_mut()
            .filter(|it| it.type_identifier == type_identifier)
            .filter(|it| it.meta_type == type_.as_ref())
            .find(|it| it.key == key);
        match found {
            Some(meta) => {
                meta.value = value.to_owned();
                true
            }
            None => {
                metas.push(MetaDomain {
                    meta_type: type_.as_ref().to_owned(),
                    type_identifier: type_identifier.to_owned(),
                    key: key.to_owned(),
                    value: value.to_owned(),
                });
                false
            }
        }
    }

    /// After every step, `Operations::metas` and `Operations::meta` find what a scan of every meta finds, for every
    /// owner and key, and the store holds what the scanning `insert_meta` built, entry by entry. The steps interleave
    /// inserts and updates of several keys over owners of the three meta types, some sharing an identifier across
    /// types. A step may repeat a key in one meta (the last value wins), update every key of an owner at once, or
    /// push to `Store::metas` directly, as tests of other crates do: a lookup still finds the entry, and a later
    /// `insert_meta` of a key the owner then has twice updates the first entry, as the scan did
    #[test]
    fn the_meta_lookups_find_what_a_scan_of_every_meta_finds() {
        let mut operations = Operations {
            timezone: Tz::UTC,
            store: Default::default(),
        };
        let types = [MetaType::AccountMeta, MetaType::CommodityMeta, MetaType::TransactionMeta];
        // identifiers used with every type: `CNY` as an account too, `budget` as an identifier and as a key
        let identifiers = ["Assets:Bank", "Assets:Bank:Card", "CNY", "a3f1c2d4", "budget"];
        let keys = ["budget", "note", "document", "rate"];
        // what the scanning `insert_meta` builds; the store must be this after every step
        let mut scanned: Vec<MetaDomain> = vec![];
        let (mut updates, mut repeated, mut whole_owner, mut direct) = (0, 0, 0, 0);

        // xorshift: the same sequence on every run
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut below = |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        for step in 0..400 {
            let type_ = &types[below(types.len())];
            let identifier = identifiers[below(identifiers.len())];
            let value = format!("v{step}");
            let mut meta = Meta::default();
            match below(10) {
                0 => {
                    // the same key twice in one meta: the first value is inserted, the second replaces it
                    let key = keys[below(keys.len())];
                    meta.insert(key.to_owned(), ZhangString::quote(format!("{value}-first")));
                    meta.insert(key.to_owned(), ZhangString::unquote(value.clone()));
                    scan_insert(&mut scanned, type_, identifier, key, &format!("{value}-first"));
                    updates += usize::from(scan_insert(&mut scanned, type_, identifier, key, &value));
                    repeated += 1;
                }
                1 => {
                    // every key the owner has, in one meta: all updates in place, so the order of the keys does not
                    // matter
                    let existing = scan(&scanned, type_, identifier).map(|it| it.key.clone()).unique().collect_vec();
                    for key in &existing {
                        meta.insert(key.clone(), ZhangString::quote(format!("{value}-{key}")));
                        updates += usize::from(scan_insert(&mut scanned, type_, identifier, key, &format!("{value}-{key}")));
                    }
                    whole_owner += usize::from(existing.len() > 1);
                }
                2 => {
                    // pushed to `Store::metas` directly, past `insert_meta`
                    let key = keys[below(keys.len())];
                    let pushed = MetaDomain {
                        meta_type: type_.as_ref().to_owned(),
                        type_identifier: identifier.to_owned(),
                        key: key.to_owned(),
                        value: value.clone(),
                    };
                    operations.write().metas.push(pushed.clone());
                    scanned.push(pushed);
                    direct += 1;
                }
                _ => {
                    let key = keys[below(keys.len())];
                    meta.insert(key.to_owned(), ZhangString::quote(value.clone()));
                    updates += usize::from(scan_insert(&mut scanned, type_, identifier, key, &value));
                }
            }
            operations.insert_meta(type_.clone(), identifier, meta).unwrap();

            assert_eq!(
                operations.read().metas.iter().map(fields).collect_vec(),
                scanned.iter().map(fields).collect_vec(),
                "the store after step {step}"
            );
            for type_ in &types {
                for identifier in identifiers.into_iter().chain(["Expenses:Food"]) {
                    let found = operations.metas(type_.clone(), identifier).unwrap();
                    assert_eq!(
                        found.iter().map(fields).collect_vec(),
                        scan(&scanned, type_, identifier).map(fields).collect_vec(),
                        "the metas of {identifier} of {} after step {step}",
                        type_.as_ref()
                    );
                    for key in keys.into_iter().chain(["payee"]) {
                        let found = operations.meta(type_.clone(), identifier, key).unwrap();
                        assert_eq!(
                            found.as_ref().map(fields),
                            scan(&scanned, type_, identifier).find(|meta| meta.key == key).map(fields),
                            "the meta {key} of {identifier} of {} after step {step}",
                            type_.as_ref()
                        );
                    }
                }
            }
        }

        assert!(updates > 0, "some steps update a key in place");
        assert!(repeated > 0 && whole_owner > 0 && direct > 0, "every kind of step occurs");
        let owners_with_several_keys = types
            .iter()
            .cartesian_product(identifiers)
            .filter(|(type_, identifier)| scan(&scanned, type_, identifier).map(|it| &it.key).unique().count() > 1)
            .count();
        assert!(owners_with_several_keys > 0, "some owners have several keys");
        let repeated_keys = scanned
            .iter()
            .map(|meta| (&meta.meta_type, &meta.type_identifier, &meta.key))
            .duplicates()
            .count();
        assert!(
            repeated_keys > 0,
            "some owner has a key twice after a direct push, so a lookup has to take the first"
        );
    }
}
