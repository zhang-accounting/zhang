//! The `postings` table: one row per posting, with the columns of its transaction.
//!
//! Rows are read directly from the ledger's in-memory [`Store`] (computed units, pads)
//! and enriched from the parsed transaction directives where the store does not keep the
//! information (price annotations, lot date and label, transaction metadata).
//!
//! Which entries produce rows follows beancount: transactions and padding transactions
//! (flag `P`) do; balance assertions (stored by zhang as transactions with flag `C`) do not.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use bigdecimal::{BigDecimal, Signed, Zero};
use chrono::{Datelike, NaiveDate};
use zhang_ast::amount::Amount;
use zhang_ast::{Directive, Flag, Meta, PostingCost, SingleTotalPrice, Transaction};
use zhang_core::domains::schemas::MetaType;
use zhang_core::inventory::BookingMethod;
use zhang_core::ledger::Ledger;
use zhang_core::store::Store;

use crate::decimal;
use crate::prices::PriceMap;
use crate::value::{Cost, DataType, Position, Value};

/// The transaction-level part of a row.
pub(crate) struct Entry<'a> {
    pub id: String,
    pub date: NaiveDate,
    pub flag: String,
    pub payee: Option<&'a str>,
    pub narration: Option<&'a str>,
    pub tags: &'a [String],
    pub links: &'a [String],
    /// metadata from the parsed directive, when it could be matched
    pub meta: Option<&'a Meta>,
    /// account of every posting, in posting order
    pub accounts: Vec<&'a str>,
}

/// One posting.
pub(crate) struct Row<'a> {
    pub entry: usize,
    pub posting_index: usize,
    pub account: &'a str,
    pub units: Cow<'a, Amount>,
    pub cost: Option<Cost>,
    /// per-unit price annotation
    pub price: Option<Cow<'a, Amount>>,
}

/// The rows of one query execution plus lazily built lookup structures.
pub(crate) struct Dataset<'a> {
    pub entries: Vec<Entry<'a>>,
    pub rows: Vec<Row<'a>>,
    pub today: NaiveDate,
    store: &'a Store,
    prices: OnceCell<PriceMap>,
    store_meta: OnceCell<HashMap<&'a str, Vec<(&'a str, &'a str)>>>,
}

/// The cost specification of a posting before lot booking.
enum CostSpec {
    None,
    Known(Cost),
    /// `{}` or a spec without a cost number: resolved against the open lots
    Incomplete {
        date: Option<NaiveDate>,
        label: Option<String>,
    },
}

struct Draft<'a> {
    entry: usize,
    posting_index: usize,
    account: &'a str,
    units: &'a Amount,
    cost: CostSpec,
    price: Option<Cow<'a, Amount>>,
}

impl<'a> Dataset<'a> {
    pub fn new(ledger: &'a Ledger, store: &'a Store, today: NaiveDate) -> Self {
        // the parsed directives, addressable by source position (the store keeps the span)
        let mut directives: HashMap<(Option<&Path>, usize), &'a Transaction> = HashMap::new();
        for directive in &ledger.directives {
            if let Directive::Transaction(txn) = &directive.data {
                directives.insert((directive.span.filename.as_deref(), directive.span.start), txn);
            }
        }

        let mut transactions = store.transactions.values().filter(|txn| txn.flag != Flag::BalanceCheck).collect::<Vec<_>>();
        transactions.sort_by_key(|txn| txn.sequence);

        let mut entries = Vec::with_capacity(transactions.len());
        let mut drafts = Vec::with_capacity(store.postings.len());
        let mut needs_booking = false;
        for txn in transactions {
            let date = txn.datetime.date_naive();
            let parsed = directives
                .get(&(txn.span.filename.as_deref(), txn.span.start))
                .copied()
                .filter(|parsed| parsed.postings.len() == txn.postings.len());
            let entry_idx = entries.len();
            entries.push(Entry {
                id: txn.id.to_string(),
                date,
                flag: txn.flag.to_string(),
                payee: txn.payee.as_deref(),
                narration: txn.narration.as_deref(),
                tags: &txn.tags,
                links: &txn.links,
                meta: parsed.map(|it| &it.meta),
                accounts: txn.postings.iter().map(|posting| posting.account.name()).collect(),
            });
            for (posting_index, posting) in txn.postings.iter().enumerate() {
                let units = &posting.inferred_amount;
                let parsed_posting = parsed.and_then(|it| it.postings.get(posting_index));
                let cost = match parsed_posting {
                    Some(parsed_posting) => cost_spec(parsed_posting.cost.as_ref(), units, date),
                    None => match &posting.cost {
                        Some(cost) => CostSpec::Known(Cost {
                            number: cost.number.clone(),
                            currency: cost.commodity.clone(),
                            date: Some(date),
                            label: None,
                        }),
                        None => CostSpec::None,
                    },
                };
                needs_booking |= matches!(cost, CostSpec::Incomplete { .. });
                let price = parsed_posting.and_then(|it| it.price.as_ref()).and_then(|price| per_unit_price(price, units));
                drafts.push(Draft {
                    entry: entry_idx,
                    posting_index,
                    account: posting.account.name(),
                    units,
                    cost,
                    price,
                });
            }
        }

        let rows = if needs_booking {
            book(drafts, ledger, store)
        } else {
            drafts
                .into_iter()
                .map(|draft| Row {
                    entry: draft.entry,
                    posting_index: draft.posting_index,
                    account: draft.account,
                    units: Cow::Borrowed(draft.units),
                    cost: match draft.cost {
                        CostSpec::Known(cost) => Some(cost),
                        _ => None,
                    },
                    price: draft.price,
                })
                .collect()
        };

        Dataset {
            entries,
            rows,
            today,
            store,
            prices: OnceCell::new(),
            store_meta: OnceCell::new(),
        }
    }

    pub fn entry(&self, row: &Row<'_>) -> &Entry<'a> {
        &self.entries[row.entry]
    }

    pub fn prices(&self) -> &PriceMap {
        self.prices.get_or_init(|| PriceMap::from_prices(&self.store.prices))
    }

    /// Posting-level metadata `key` of the row, as a string.
    ///
    /// This is the single seam for posting metadata: zhang-core does not keep posting-level
    /// metadata yet (metadata lines under a posting are folded into the transaction), so it
    /// always returns `None` for now. Once the store records metadata per posting (keyed by
    /// the posting id, `Uuid::from_txn_posting(trx_id, row.posting_index)`), look it up here
    /// and `meta(key)` starts returning values without any API change. See issue #434.
    pub fn posting_meta(&self, _row: &Row<'_>, _key: &str) -> Option<String> {
        None
    }

    /// Transaction metadata `key` of the row, as a string.
    pub fn entry_meta(&self, row: &Row<'_>, key: &str) -> Option<String> {
        let entry = self.entry(row);
        if let Some(meta) = entry.meta {
            return meta.get_one(key).map(|value| value.as_str().to_owned());
        }
        let index = self.store_meta.get_or_init(|| {
            let mut index: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
            for meta in &self.store.metas {
                if meta.meta_type == MetaType::TransactionMeta.as_ref() {
                    index
                        .entry(meta.type_identifier.as_str())
                        .or_default()
                        .push((meta.key.as_str(), meta.value.as_str()));
                }
            }
            index
        });
        index
            .get(entry.id.as_str())
            .and_then(|pairs| pairs.iter().find(|(k, _)| *k == key))
            .map(|(_, value)| (*value).to_owned())
    }
}

fn cost_spec(cost: Option<&PostingCost>, units: &Amount, txn_date: NaiveDate) -> CostSpec {
    let Some(cost) = cost else {
        return CostSpec::None;
    };
    let date = cost.date.as_ref().map(|it| it.naive_date());
    match &cost.base {
        Some(base) => {
            let number = if cost.total {
                decimal::div(&base.number, &units.number.abs()).unwrap_or_else(|| base.number.clone())
            } else {
                base.number.clone()
            };
            CostSpec::Known(Cost {
                number,
                currency: base.commodity.clone(),
                // like zhang's lot booking (and beancount's augmentations), a lot without an
                // explicit date is dated by its transaction
                date: date.or(Some(txn_date)),
                label: cost.label.clone(),
            })
        }
        None => CostSpec::Incomplete {
            date,
            label: cost.label.clone(),
        },
    }
}

fn per_unit_price<'a>(price: &'a SingleTotalPrice, units: &Amount) -> Option<Cow<'a, Amount>> {
    match price {
        SingleTotalPrice::Single(price) => Some(Cow::Borrowed(price)),
        SingleTotalPrice::Total(total) => {
            decimal::div(&total.number, &units.number.abs()).map(|number| Cow::Owned(Amount::new(number, total.commodity.clone())))
        }
    }
}

/// Resolve `{}`-style reductions against the lots opened by earlier postings of the same
/// account and currency, splitting a reduction that spans several lots into one row per
/// lot (as beancount's booking does). Lots are consumed FIFO, or LIFO when the account
/// (or the ledger default) uses the LIFO booking method.
fn book<'a>(drafts: Vec<Draft<'a>>, ledger: &Ledger, store: &Store) -> Vec<Row<'a>> {
    let mut account_methods: HashMap<&str, BookingMethod> = HashMap::new();
    for meta in &store.metas {
        if meta.meta_type == MetaType::AccountMeta.as_ref() && meta.key == "booking_method" {
            if let Ok(method) = meta.value.parse::<BookingMethod>() {
                account_methods.insert(meta.type_identifier.as_str(), method);
            }
        }
    }
    let default_method = ledger.options.default_booking_method;

    let mut lots: HashMap<(&str, &str), Vec<(Cost, BigDecimal)>> = HashMap::new();
    let mut rows = Vec::with_capacity(drafts.len());
    for draft in drafts {
        let key = (draft.account, draft.units.commodity.as_str());
        match draft.cost {
            CostSpec::None => rows.push(Row {
                entry: draft.entry,
                posting_index: draft.posting_index,
                account: draft.account,
                units: Cow::Borrowed(draft.units),
                cost: None,
                price: draft.price,
            }),
            CostSpec::Known(cost) => {
                let account_lots = lots.entry(key).or_default();
                match account_lots.iter_mut().find(|(lot, _)| *lot == cost) {
                    Some((_, number)) => *number += &draft.units.number,
                    None => account_lots.push((cost.clone(), draft.units.number.clone())),
                }
                account_lots.retain(|(_, number)| !number.is_zero());
                rows.push(Row {
                    entry: draft.entry,
                    posting_index: draft.posting_index,
                    account: draft.account,
                    units: Cow::Borrowed(draft.units),
                    cost: Some(cost),
                    price: draft.price,
                });
            }
            CostSpec::Incomplete { date, label } => {
                let lifo = matches!(account_methods.get(draft.account).copied().unwrap_or(default_method), BookingMethod::Lifo);
                let account_lots = lots.entry(key).or_default();
                let mut remaining = draft.units.number.clone();
                let mut order = (0..account_lots.len()).collect::<Vec<_>>();
                if lifo {
                    order.reverse();
                }
                for idx in order {
                    if remaining.is_zero() {
                        break;
                    }
                    let (lot, number) = &mut account_lots[idx];
                    let matches_spec = number.is_positive() != remaining.is_positive()
                        && date.map(|date| lot.date == Some(date)).unwrap_or(true)
                        && label.as_ref().map(|label| lot.label.as_ref() == Some(label)).unwrap_or(true);
                    if !matches_spec {
                        continue;
                    }
                    let take = if remaining.abs() >= number.abs() {
                        -number.clone()
                    } else {
                        remaining.clone()
                    };
                    *number += &take;
                    remaining -= &take;
                    rows.push(Row {
                        entry: draft.entry,
                        posting_index: draft.posting_index,
                        account: draft.account,
                        units: Cow::Owned(Amount::new(take, draft.units.commodity.clone())),
                        cost: Some(lot.clone()),
                        price: draft.price.clone(),
                    });
                }
                account_lots.retain(|(_, number)| !number.is_zero());
                if !remaining.is_zero() {
                    let units = if remaining == draft.units.number {
                        Cow::Borrowed(draft.units)
                    } else {
                        Cow::Owned(Amount::new(remaining, draft.units.commodity.clone()))
                    };
                    rows.push(Row {
                        entry: draft.entry,
                        posting_index: draft.posting_index,
                        account: draft.account,
                        units,
                        cost: None,
                        price: draft.price,
                    });
                }
            }
        }
    }
    rows
}

/// A column of the `postings` table.
pub struct ColumnDef {
    pub name: &'static str,
    pub ty: DataType,
    pub description: &'static str,
    pub(crate) get: fn(&Dataset<'_>, &Row<'_>) -> Value,
}

impl std::fmt::Debug for ColumnDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.ty)
    }
}

/// The columns produced by `SELECT *`.
pub(crate) const WILDCARD_COLUMNS: [&str; 6] = ["date", "flag", "payee", "narration", "account", "position"];

fn set_of(items: &[String]) -> Value {
    Value::Set(items.iter().cloned().collect::<BTreeSet<_>>())
}

fn opt_str(value: Option<&str>) -> Value {
    value.map(|it| Value::Str(it.to_owned())).unwrap_or(Value::Null)
}

fn weight(row: &Row<'_>) -> Amount {
    match (&row.cost, &row.price) {
        (Some(cost), _) => Amount::new(decimal::mul(&row.units.number, &cost.number), cost.currency.clone()),
        (None, Some(price)) => Amount::new(decimal::mul(&row.units.number, &price.number), price.commodity.clone()),
        (None, None) => row.units.as_ref().clone(),
    }
}

/// The `postings` table columns.
pub static COLUMNS: &[ColumnDef] = &[
    ColumnDef {
        name: "date",
        ty: DataType::Date,
        description: "Date of the transaction.",
        get: |data, row| Value::Date(data.entry(row).date),
    },
    ColumnDef {
        name: "year",
        ty: DataType::Int,
        description: "Year of the transaction date.",
        get: |data, row| Value::Int(data.entry(row).date.year() as i64),
    },
    ColumnDef {
        name: "month",
        ty: DataType::Int,
        description: "Month (1-12) of the transaction date.",
        get: |data, row| Value::Int(data.entry(row).date.month() as i64),
    },
    ColumnDef {
        name: "day",
        ty: DataType::Int,
        description: "Day of month of the transaction date.",
        get: |data, row| Value::Int(data.entry(row).date.day() as i64),
    },
    ColumnDef {
        name: "flag",
        ty: DataType::Str,
        description: "Flag of the transaction: '*', '!', or 'P' for padding.",
        get: |data, row| Value::Str(data.entry(row).flag.clone()),
    },
    ColumnDef {
        name: "payee",
        ty: DataType::Str,
        description: "Payee of the transaction.",
        get: |data, row| opt_str(data.entry(row).payee),
    },
    ColumnDef {
        name: "narration",
        ty: DataType::Str,
        description: "Narration of the transaction; '' when absent (as in beancount).",
        get: |data, row| Value::Str(data.entry(row).narration.unwrap_or_default().to_owned()),
    },
    ColumnDef {
        name: "description",
        ty: DataType::Str,
        description: "Payee and narration joined with ' | ' (whichever are present).",
        get: |data, row| {
            let entry = data.entry(row);
            let parts = [entry.payee, entry.narration]
                .into_iter()
                .flatten()
                .filter(|it| !it.is_empty())
                .collect::<Vec<_>>();
            Value::Str(parts.join(" | "))
        },
    },
    ColumnDef {
        name: "tags",
        ty: DataType::Set,
        description: "Tags of the transaction.",
        get: |data, row| set_of(data.entry(row).tags),
    },
    ColumnDef {
        name: "links",
        ty: DataType::Set,
        description: "Links of the transaction.",
        get: |data, row| set_of(data.entry(row).links),
    },
    ColumnDef {
        name: "id",
        ty: DataType::Str,
        description: "Unique id of the transaction.",
        get: |data, row| Value::Str(data.entry(row).id.clone()),
    },
    ColumnDef {
        name: "account",
        ty: DataType::Str,
        description: "Account of the posting.",
        get: |_, row| Value::Str(row.account.to_owned()),
    },
    ColumnDef {
        name: "number",
        ty: DataType::Decimal,
        description: "Number of units of the posting.",
        get: |_, row| Value::Decimal(row.units.number.clone()),
    },
    ColumnDef {
        name: "currency",
        ty: DataType::Str,
        description: "Currency of the units of the posting.",
        get: |_, row| Value::Str(row.units.commodity.clone()),
    },
    ColumnDef {
        name: "position",
        ty: DataType::Position,
        description: "Units and cost of the posting.",
        get: |_, row| Value::Position(Position::new(row.units.as_ref().clone(), row.cost.clone())),
    },
    ColumnDef {
        name: "cost_number",
        ty: DataType::Decimal,
        description: "Per-unit cost number of the posting's lot.",
        get: |_, row| row.cost.as_ref().map(|cost| Value::Decimal(cost.number.clone())).unwrap_or(Value::Null),
    },
    ColumnDef {
        name: "cost_currency",
        ty: DataType::Str,
        description: "Cost currency of the posting's lot.",
        get: |_, row| row.cost.as_ref().map(|cost| Value::Str(cost.currency.clone())).unwrap_or(Value::Null),
    },
    ColumnDef {
        name: "cost_date",
        ty: DataType::Date,
        description: "Acquisition date of the posting's lot.",
        get: |_, row| row.cost.as_ref().and_then(|cost| cost.date).map(Value::Date).unwrap_or(Value::Null),
    },
    ColumnDef {
        name: "cost_label",
        ty: DataType::Str,
        description: "Label of the posting's lot; '' when the posting has no cost (as in beanquery).",
        get: |_, row| match &row.cost {
            None => Value::Str(String::new()),
            Some(cost) => opt_str(cost.label.as_deref()),
        },
    },
    ColumnDef {
        name: "price",
        ty: DataType::Amount,
        description: "Per-unit price annotation (@ or @@) of the posting.",
        get: |_, row| row.price.as_ref().map(|price| Value::Amount(price.as_ref().clone())).unwrap_or(Value::Null),
    },
    ColumnDef {
        name: "weight",
        ty: DataType::Amount,
        description: "Amount the posting contributes to the transaction balance: units × cost, else units × price, else units.",
        get: |_, row| Value::Amount(weight(row)),
    },
    ColumnDef {
        name: "other_accounts",
        ty: DataType::Set,
        description: "Accounts of the other postings of the transaction.",
        get: |data, row| {
            let entry = data.entry(row);
            Value::Set(
                entry
                    .accounts
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| *idx != row.posting_index)
                    .map(|(_, account)| (*account).to_owned())
                    .collect(),
            )
        },
    },
];

pub(crate) fn column(name: &str) -> Option<&'static ColumnDef> {
    COLUMNS.iter().find(|it| it.name.eq_ignore_ascii_case(name))
}
