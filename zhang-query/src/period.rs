//! The accounting-period modifiers of the FROM clause: `OPEN ON`, `CLOSE [ON]` and `CLEAR`.
//!
//! `FROM [expr] [OPEN ON date] [CLOSE [ON date]] [CLEAR]` rewrites the booked postings before
//! any filter sees them, with beancount's summarize, truncate and clear operations, applied as
//! beanquery applies them: OPEN, then CLOSE, then CLEAR, over every posting of the ledger. The
//! FROM expression and WHERE then filter the rewritten rows (in beanquery 0.2 the FROM
//! expression is a posting filter like WHERE, evaluated after the period modifiers).
//!
//! - **`OPEN ON d`** summarizes everything before `d` into opening balances dated the day
//!   before `d`. First, what price conversions (`@`) left unbalanced at cost before `d` is
//!   booked to the previous-conversions account. Then the income and expense balances are
//!   transferred to the previous-earnings account. Finally, every account with a balance gets
//!   one summarization entry (flag `S`) with a posting per lot (units and cost, so holdings
//!   keep their lots) against the opening-balances account, which receives the lot's cost.
//! - **`CLOSE ON d`** drops everything on or after `d` and, when price conversions leave the
//!   period unbalanced at cost, adds a conversion entry (flag `C`) dated the day before `d` on
//!   the current-conversions account, priced at zero in the conversion currency. A bare
//!   **`CLOSE`** drops nothing and dates the conversion entry like the last entry.
//! - **`CLEAR`** transfers the income and expense balances to the current-earnings account with
//!   one entry per account (flag `T`), dated like the last entry of the period.
//!
//! Balances are summed per account and lot from the booked rows, so lot booking runs over the
//! whole ledger first (see [`crate::table`]) and summarized lots keep their cost, date and
//! label. Within an entry, postings follow the lots in the order they were opened, accounts are
//! sorted by name, and synthetic entries have no payee, tags, links or metadata, as in
//! beancount.
//!
//! # Equity accounts
//!
//! zhang has no options of its own for these accounts. Its store keeps every `option`
//! directive verbatim, so beancount's options are honoured; each names a leaf under `Equity`:
//!
//! | option | default |
//! |---|---|
//! | `account_previous_balances` | `Equity:Opening-Balances` |
//! | `account_previous_earnings` | `Equity:Earnings:Previous` |
//! | `account_previous_conversions` | `Equity:Conversions:Previous` |
//! | `account_current_earnings` | `Equity:Earnings:Current` |
//! | `account_current_conversions` | `Equity:Conversions:Current` |
//! | `conversion_currency` | `NOTHING` |
//!
//! An option value that is not a valid leaf account name falls back to the default.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;

use bigdecimal::{BigDecimal, Zero};
use chrono::{NaiveDate, NaiveTime};
use indexmap::IndexMap;
use uuid::Uuid;
use zhang_ast::amount::Amount;
use zhang_ast::{resolve_local_datetime, Account, AccountType, Directive, Flag, SpanInfo};
use zhang_core::ledger::Ledger;
use zhang_core::store::{PostingDomain, TransactionDomain};
use zhang_core::utils::id::FromSpan;

use crate::error::{LocatedError, Span};
use crate::params::{ParamRef, Params};
use crate::table::{Dataset, Entry, MaybeOwned, Row};
use crate::value::{Cost, Position, Value};

/// A date of the period modifiers: a literal of the query or a parameter bound at execution.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PeriodDate {
    Fixed(NaiveDate),
    Param(ParamRef, Span),
}

impl PeriodDate {
    fn resolve(&self, params: &Params, clause: &str) -> Result<NaiveDate, LocatedError> {
        match self {
            PeriodDate::Fixed(date) => Ok(*date),
            PeriodDate::Param(param, span) => match params.get(param) {
                Some(Value::Date(date)) => Ok(*date),
                _ => Err(LocatedError::eval(
                    format!("{} needs a date, but parameter {} is NULL", clause, param),
                    Some(*span),
                )),
            },
        }
    }
}

impl fmt::Display for PeriodDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeriodDate::Fixed(date) => write!(f, "{}", date),
            PeriodDate::Param(param, _) => write!(f, "{}", param),
        }
    }
}

/// The compiled period modifiers of a query.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Period {
    pub open: Option<PeriodDate>,
    /// `Some(None)` for a bare `CLOSE`
    pub close: Option<Option<PeriodDate>>,
    pub clear: bool,
}

impl Period {
    /// The dates of one execution.
    pub fn resolve(&self, params: &Params) -> Result<ResolvedPeriod, LocatedError> {
        let open = self.open.as_ref().map(|date| date.resolve(params, "OPEN ON")).transpose()?;
        let close = match &self.close {
            None => None,
            Some(None) => Some(None),
            Some(Some(date)) => Some(Some(date.resolve(params, "CLOSE ON")?)),
        };
        if let (Some(open), Some(Some(close))) = (open, close) {
            if close < open {
                let span = match &self.close {
                    Some(Some(PeriodDate::Param(_, span))) => Some(*span),
                    _ => None,
                };
                return Err(LocatedError::eval(format!("the CLOSE date {} is before the OPEN date {}", close, open), span));
            }
        }
        Ok(ResolvedPeriod {
            open,
            close,
            clear: self.clear,
        })
    }
}

/// `OPEN ON 2024-01-01 CLOSE ON :to CLEAR`
impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = vec![];
        if let Some(open) = &self.open {
            parts.push(format!("OPEN ON {}", open));
        }
        match &self.close {
            None => {}
            Some(None) => parts.push("CLOSE".to_owned()),
            Some(Some(close)) => parts.push(format!("CLOSE ON {}", close)),
        }
        if self.clear {
            parts.push("CLEAR".to_owned());
        }
        f.write_str(&parts.join(" "))
    }
}

/// The period modifiers with their dates known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedPeriod {
    pub open: Option<NaiveDate>,
    /// `Some(None)` for a bare `CLOSE`
    pub close: Option<Option<NaiveDate>>,
    pub clear: bool,
}

/// The accounts the period modifiers post to, and the currency of the conversion price.
#[derive(Debug, Clone)]
pub(crate) struct EquityAccounts {
    opening_balances: Account,
    previous_earnings: Account,
    previous_conversions: Account,
    current_earnings: Account,
    current_conversions: Account,
    conversion_currency: String,
}

impl EquityAccounts {
    /// Read beancount's options from the ledger's options, with beancount's defaults.
    pub fn from_options(options: &HashMap<String, String>) -> EquityAccounts {
        let account = |key: &str, default: &str| {
            let leaf = options
                .get(key)
                .map(|value| value.trim())
                .filter(|leaf| is_leaf_account(leaf))
                .unwrap_or(default);
            Account {
                account_type: AccountType::Equity,
                content: format!("Equity:{}", leaf),
                components: leaf.split(':').map(str::to_owned).collect(),
            }
        };
        let conversion_currency = options
            .get("conversion_currency")
            .map(|value| value.trim())
            .filter(|currency| !currency.is_empty() && !currency.contains(char::is_whitespace))
            .unwrap_or("NOTHING");
        EquityAccounts {
            opening_balances: account("account_previous_balances", "Opening-Balances"),
            previous_earnings: account("account_previous_earnings", "Earnings:Previous"),
            previous_conversions: account("account_previous_conversions", "Conversions:Previous"),
            current_earnings: account("account_current_earnings", "Earnings:Current"),
            current_conversions: account("account_current_conversions", "Conversions:Current"),
            conversion_currency: conversion_currency.to_owned(),
        }
    }
}

/// `Earnings:Previous`: non-empty components without whitespace, separated by `:`.
fn is_leaf_account(leaf: &str) -> bool {
    leaf.split(':')
        .all(|component| !component.is_empty() && !component.contains(char::is_whitespace))
}

/// Income statement accounts are the ones whose balances CLEAR and OPEN transfer to equity.
fn is_income_statement(account: &str) -> bool {
    matches!(account.split(':').next(), Some("Income" | "Expenses"))
}

fn previous_day(date: NaiveDate) -> NaiveDate {
    date.pred_opt().unwrap_or(date)
}

/// Lots in the order they were first added, like beancount's inventories: the postings of a
/// summarization or transfer entry follow this order. A lot that nets to zero is dropped, so
/// adding it again later puts it last.
#[derive(Debug, Default)]
struct Lots {
    lots: IndexMap<(String, Option<Cost>), BigDecimal>,
}

impl Lots {
    fn add(&mut self, units: &Amount, cost: Option<&Cost>) {
        let key = (units.commodity.clone(), cost.cloned());
        match self.lots.get_mut(&key) {
            Some(number) => {
                *number += &units.number;
                if number.is_zero() {
                    self.lots.shift_remove(&key);
                }
            }
            None if !units.number.is_zero() => {
                self.lots.insert(key, units.number.clone());
            }
            None => {}
        }
    }

    fn positions(&self) -> impl Iterator<Item = Position> + '_ {
        self.lots
            .iter()
            .map(|((currency, cost), number)| Position::new(Amount::new(number.clone(), currency.clone()), cost.clone()))
    }

    fn is_empty(&self) -> bool {
        self.lots.is_empty()
    }

    /// The cost of every lot (the units of lots without cost), merged per currency in order.
    fn at_cost(&self) -> Lots {
        let mut reduced = Lots::default();
        for position in self.positions() {
            reduced.add(&position.at_cost(), None);
        }
        reduced
    }
}

/// The positions as beancount prints an inventory: by currency rank (the major currencies
/// USD, EUR, JPY, CAD, GBP, AUD, NZD and CHF in this order, then the others by the length of
/// their name), then by cost number (zero without a cost), cost currency and units; ties keep
/// their order. Observed from beancount's output: the currency name, cost date and label do not
/// take part.
fn beancount_order(mut positions: Vec<Position>) -> Vec<String> {
    const MAJOR: [&str; 8] = ["USD", "EUR", "JPY", "CAD", "GBP", "AUD", "NZD", "CHF"];
    let rank = |position: &Position| {
        let currency = position.units.commodity.as_str();
        MAJOR.iter().position(|it| *it == currency).unwrap_or(MAJOR.len() + currency.chars().count())
    };
    let zero = BigDecimal::zero();
    let cost_number = |position: &Position| position.cost.as_ref().map_or(&zero, |cost| &cost.number).clone();
    let cost_currency = |position: &Position| position.cost.as_ref().map_or("", |cost| cost.currency.as_str()).to_owned();
    positions.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| cost_number(a).cmp(&cost_number(b)))
            .then_with(|| cost_currency(a).cmp(&cost_currency(b)))
            .then_with(|| a.units.number.cmp(&b.units.number))
    });
    positions.iter().map(ToString::to_string).collect()
}

/// The balance of one account.
struct Balance {
    /// the account as stored, for the postings of synthetic entries
    account: Account,
    lots: Lots,
}

/// The kinds of synthetic entries, with beancount's flags and metadata file names.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Summarize,
    Transfer,
    Conversion,
}

impl Kind {
    fn flag(self) -> &'static str {
        match self {
            Kind::Summarize => "S",
            Kind::Transfer => "T",
            Kind::Conversion => "C",
        }
    }

    /// the pseudo source of the entry, which its id is derived from
    fn source(self) -> &'static str {
        match self {
            Kind::Summarize => "<summarize>",
            Kind::Transfer => "<transfer_balances>",
            Kind::Conversion => "<conversions>",
        }
    }
}

/// One posting of a synthetic entry.
struct Posting<'a> {
    name: &'a str,
    account: Account,
    units: Amount,
    cost: Option<Cost>,
    price: Option<Amount>,
}

impl ResolvedPeriod {
    /// Rewrite the rows of `data`. Its rows must carry the cost of their lots
    /// ([`crate::projector::Projection::with_cost`]). The account balances are summed over the
    /// rewritten rows, as the executor sums them over the rows of the table.
    pub fn apply<'a>(&self, mut data: Dataset<'a>, ledger: &'a Ledger, equity: &'a EquityAccounts) -> Dataset<'a> {
        let keep_price = data.projection.keeps_price();
        let rows = std::mem::take(&mut data.rows);
        let mut transform = Transform {
            entries: &mut data.entries,
            rows,
            ledger,
            equity,
            keep_price,
        };
        if let Some(open) = self.open {
            transform.open(open);
        }
        let close_on = self.close.flatten();
        if let Some(close) = self.close {
            transform.close(close, self.open);
        }
        if self.clear {
            transform.clear(self.open, close_on);
        }
        data.rows = transform.rows;
        data
    }
}

struct Transform<'d, 'a> {
    entries: &'d mut Vec<Entry<'a>>,
    /// the rows of the period so far, in entry order
    rows: Vec<Row<'a>>,
    ledger: &'a Ledger,
    equity: &'a EquityAccounts,
    /// whether rows keep their price annotation (the zero price of conversion postings)
    keep_price: bool,
}

impl<'a> Transform<'_, 'a> {
    fn date(&self, row: &Row<'_>) -> NaiveDate {
        self.entries[row.entry].date
    }

    /// The balance of every account over `rows`, sorted by account name.
    fn balances(&self, rows: &[Row<'a>]) -> BTreeMap<&'a str, Balance> {
        let mut balances: BTreeMap<&'a str, Balance> = BTreeMap::new();
        for row in rows {
            balances
                .entry(row.account)
                .or_insert_with(|| Balance {
                    account: self.entries[row.entry].txn.postings[row.posting_index].account.clone(),
                    lots: Lots::default(),
                })
                .lots
                .add(&row.units, row.cost.as_deref());
        }
        balances
    }

    /// The balance of the equity account `account` (created empty).
    fn equity_balance<'b>(balances: &'b mut BTreeMap<&'a str, Balance>, account: &'a Account) -> &'b mut Balance {
        balances.entry(account.name()).or_insert_with(|| Balance {
            account: account.clone(),
            lots: Lots::default(),
        })
    }

    /// The sum of every posting of `rows` by lot: what remains when price conversions leave
    /// the rows unbalanced (beancount's conversion balance).
    fn conversion_balance(rows: &[Row<'a>]) -> Lots {
        let mut balance = Lots::default();
        for row in rows {
            balance.add(&row.units, row.cost.as_deref());
        }
        balance
    }

    /// The date of the last entry of the period: the latest of its rows and of the ledger's
    /// other dated directives between the OPEN date (inclusive) and the CLOSE date (exclusive).
    /// zhang's budget directives have no beancount counterpart and do not count.
    fn last_date(&self, open: Option<NaiveDate>, close: Option<NaiveDate>) -> Option<NaiveDate> {
        let rows = self.rows.iter().map(|row| self.date(row));
        let directives = self
            .ledger
            .directives
            .iter()
            .filter(|directive| {
                !matches!(
                    directive.data,
                    Directive::Budget(_) | Directive::BudgetAdd(_) | Directive::BudgetTransfer(_) | Directive::BudgetClose(_)
                )
            })
            .filter_map(|directive| directive.data.datetime())
            .map(|datetime| datetime.date())
            .filter(|date| open.is_none_or(|open| *date >= open) && close.is_none_or(|close| *date < close));
        rows.chain(directives).max()
    }

    /// OPEN ON `date`: summarize the rows before `date` into opening balances.
    fn open(&mut self, date: NaiveDate) {
        let rows = std::mem::take(&mut self.rows);
        let (before, after): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| self.date(row) < date);
        let mut balances = self.balances(&before);

        // the conversions before the period, as an entry dated the day before
        let conversions = Self::conversion_balance(&before).at_cost();
        let previous_conversions = Self::equity_balance(&mut balances, &self.equity.previous_conversions);
        for position in conversions.positions() {
            previous_conversions.lots.add(&-position.units, None);
        }

        // the income and expenses before the period move to the previous earnings
        let mut transferred = vec![];
        for (name, balance) in balances.iter_mut() {
            if is_income_statement(name) {
                transferred.extend(balance.lots.positions());
                balance.lots = Lots::default();
            }
        }
        let previous_earnings = Self::equity_balance(&mut balances, &self.equity.previous_earnings);
        for position in transferred {
            previous_earnings.lots.add(&position.at_cost(), None);
        }

        // one summarization entry per account with a balance
        let eve = previous_day(date);
        let opening = &self.equity.opening_balances;
        let mut rows = vec![];
        for (name, balance) in balances {
            if balance.lots.is_empty() {
                continue;
            }
            let mut postings = vec![];
            for position in balance.lots.positions() {
                let at_cost = position.at_cost();
                postings.push(Posting {
                    name,
                    account: balance.account.clone(),
                    units: position.units,
                    cost: position.cost,
                    price: None,
                });
                postings.push(Posting {
                    name: opening.name(),
                    account: opening.clone(),
                    units: -at_cost,
                    cost: None,
                    price: None,
                });
            }
            let narration = format!("Opening balance for '{}' (Summarization)", name);
            self.push_entry(&mut rows, Kind::Summarize, eve, narration, postings);
        }
        rows.extend(after);
        self.rows = rows;
    }

    /// CLOSE [ON `date`]: drop the rows on or after `date`, then balance the conversions.
    fn close(&mut self, date: Option<NaiveDate>, open: Option<NaiveDate>) {
        if let Some(date) = date {
            let entries = &self.entries;
            self.rows.retain(|row| entries[row.entry].date < date);
        }
        let balance = Self::conversion_balance(&self.rows);
        let conversions = balance.at_cost();
        if conversions.is_empty() {
            return;
        }
        let Some(entry_date) = date.map(previous_day).or_else(|| self.last_date(open, None)) else {
            return;
        };
        let account = &self.equity.current_conversions;
        let price = Amount::new(BigDecimal::zero(), self.equity.conversion_currency.clone());
        let postings = conversions
            .positions()
            .map(|position| Posting {
                name: account.name(),
                account: account.clone(),
                units: -position.units,
                cost: None,
                price: Some(price.clone()),
            })
            .collect();
        let narration = format!("Conversion for ({})", beancount_order(balance.positions().collect()).join(", "));
        let mut rows = std::mem::take(&mut self.rows);
        self.push_entry(&mut rows, Kind::Conversion, entry_date, narration, postings);
        self.rows = rows;
    }

    /// CLEAR: transfer the income and expense balances to the current earnings.
    fn clear(&mut self, open: Option<NaiveDate>, close: Option<NaiveDate>) {
        if self.rows.is_empty() {
            return;
        }
        let Some(date) = self.last_date(open, close) else {
            return;
        };
        let balances = self.balances(&self.rows);
        let earnings = &self.equity.current_earnings;
        let mut rows = std::mem::take(&mut self.rows);
        for (name, balance) in balances {
            if !is_income_statement(name) || balance.lots.is_empty() {
                continue;
            }
            let mut postings = vec![];
            for position in balance.lots.positions() {
                let at_cost = position.at_cost();
                postings.push(Posting {
                    name,
                    account: balance.account.clone(),
                    units: -position.units,
                    cost: position.cost,
                    price: None,
                });
                postings.push(Posting {
                    name: earnings.name(),
                    account: earnings.clone(),
                    units: at_cost,
                    cost: None,
                    price: None,
                });
            }
            let narration = format!("Transfer balance for '{}' (Transfer balance)", name);
            self.push_entry(&mut rows, Kind::Transfer, date, narration, postings);
        }
        self.rows = rows;
    }

    /// Add a synthetic entry with `postings` and append its rows to `rows`.
    fn push_entry(&mut self, rows: &mut Vec<Row<'a>>, kind: Kind, date: NaiveDate, narration: String, postings: Vec<Posting<'a>>) {
        let entry = self.entries.len();
        let id = Uuid::from_span(&SpanInfo {
            start: entry,
            end: 0,
            content: String::new(),
            filename: Some(PathBuf::from(kind.source())),
        });
        let datetime = resolve_local_datetime(&self.ledger.options.timezone, &date.and_time(NaiveTime::MIN));
        let mut stored = Vec::with_capacity(postings.len());
        for (posting_index, posting) in postings.into_iter().enumerate() {
            stored.push(PostingDomain {
                id: Uuid::from_txn_posting(&id, posting_index),
                trx_id: id,
                trx_sequence: 0,
                trx_datetime: datetime,
                flag: None,
                account: posting.account,
                unit: Some(posting.units.clone()),
                cost: posting.cost.as_ref().map(|cost| Amount::new(cost.number.clone(), cost.currency.clone())),
                inferred_amount: posting.units.clone(),
                // running balances are not tracked for synthetic postings
                previous_amount: Amount::zero(posting.units.commodity.clone()),
                after_amount: Amount::zero(posting.units.commodity.clone()),
                metas: vec![],
            });
            rows.push(Row {
                entry,
                posting_index,
                account: posting.name,
                units: MaybeOwned::owned(posting.units),
                cost: posting.cost.map(MaybeOwned::owned),
                price: posting.price.filter(|_| self.keep_price).map(MaybeOwned::owned),
            });
        }
        self.entries.push(Entry {
            txn: MaybeOwned::owned(TransactionDomain {
                id,
                sequence: 0,
                datetime,
                flag: Flag::Custom(kind.flag().to_owned()),
                payee: None,
                narration: Some(narration),
                span: SpanInfo::default(),
                tags: vec![],
                links: vec![],
                postings: stored,
            }),
            date,
            meta: None,
            seq: None,
            errors: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use zhang_core::data_source::LocalFileSystemDataSource;
    use zhang_core::data_type::text::ZhangDataType;

    use super::*;
    use crate::executor::execute;
    use crate::projector::Projection;
    use crate::table::COLUMNS;
    use crate::Query;

    /// Run `sql` (with period modifiers) over a dataset built for `projection` (or the query's
    /// own projection).
    fn run(ledger: &Ledger, sql: &str, projection: Option<Projection>) -> String {
        let query = Query::compile(sql).unwrap_or_else(|err| panic!("{sql}: {err}"));
        let store = ledger.store.read().unwrap();
        let equity = EquityAccounts::from_options(&store.options);
        let period = query.plan.period.as_ref().expect("a period query").resolve(&Params::new()).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let data = Dataset::new(ledger, &store, today, projection.unwrap_or(query.projection).with_cost());
        let data = period.apply(data, ledger, &equity);
        let rows = execute(&query.plan, &data, &Params::new(), None).unwrap_or_else(|err| panic!("{sql}: {}", err.message));
        // the Debug form keeps decimal scales, so equal strings are identical results
        format!("{rows:?}")
    }

    #[test]
    fn pruning_keeps_results_with_period_modifiers() {
        let source = LocalFileSystemDataSource::new(ZhangDataType {});
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../integration-tests/fava-demo-ledger");
        let ledger = Ledger::load_with_data_source(dir, "main.zhang".to_owned(), Arc::new(source)).expect("cannot load ledger");
        let queries = [
            "SELECT * FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR",
            "SELECT date, flag, account, position, price, weight, other_accounts, id FROM OPEN ON 2016-01-01 CLOSE CLEAR WHERE flag != '*'",
            "SELECT account, sum(number), count(*) FROM OPEN ON 2016-06-01 CLOSE GROUP BY 1 ORDER BY 1",
            "SELECT flag, sum(weight), sum(cost(position)) FROM year = 2016 CLOSE ON 2017-01-01 CLEAR GROUP BY 1 ORDER BY 1",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(
            COLUMNS
                .iter()
                .map(|column| format!("SELECT {} FROM OPEN ON 2016-01-01 CLOSE ON 2017-01-01 CLEAR", column.name)),
        );
        for sql in queries {
            assert_eq!(run(&ledger, &sql, None), run(&ledger, &sql, Some(Projection::all())), "{sql}");
        }
    }

    #[test]
    fn equity_accounts_default_to_beancount_names() {
        let equity = EquityAccounts::from_options(&HashMap::new());
        assert_eq!(equity.opening_balances.name(), "Equity:Opening-Balances");
        assert_eq!(equity.previous_earnings.name(), "Equity:Earnings:Previous");
        assert_eq!(equity.previous_conversions.name(), "Equity:Conversions:Previous");
        assert_eq!(equity.current_earnings.name(), "Equity:Earnings:Current");
        assert_eq!(equity.current_conversions.name(), "Equity:Conversions:Current");
        assert_eq!(equity.conversion_currency, "NOTHING");
    }

    #[test]
    fn equity_accounts_read_beancount_options() {
        let options = [
            ("account_previous_balances", "Opening"),
            ("account_previous_earnings", "Retained:Earnings"),
            ("account_current_earnings", " not valid "),
            ("account_current_conversions", "Bad::Leaf"),
            ("conversion_currency", "ZERO"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let equity = EquityAccounts::from_options(&options);
        assert_eq!(equity.opening_balances.name(), "Equity:Opening");
        assert_eq!(equity.previous_earnings.name(), "Equity:Retained:Earnings");
        assert_eq!(equity.previous_earnings.components, vec!["Retained", "Earnings"]);
        assert_eq!(equity.current_earnings.name(), "Equity:Earnings:Current");
        assert_eq!(equity.current_conversions.name(), "Equity:Conversions:Current");
        assert_eq!(equity.conversion_currency, "ZERO");
    }

    #[test]
    fn lots_keep_insertion_order_and_drop_zero_lots() {
        let usd = |number: i32| Amount::new(BigDecimal::from(number), "USD");
        let eur = |number: i32| Amount::new(BigDecimal::from(number), "EUR");
        let mut lots = Lots::default();
        lots.add(&usd(1), None);
        lots.add(&eur(2), None);
        lots.add(&usd(-1), None);
        lots.add(&usd(3), None);
        let positions = lots.positions().map(|it| it.units).collect::<Vec<_>>();
        assert_eq!(positions, vec![eur(2), usd(3)]);
    }

    #[test]
    fn orders_positions_like_beancount() {
        let position = |number: &str, currency: &str, cost: Option<(&str, &str)>| {
            let cost = cost.map(|(number, currency)| Cost {
                number: number.parse().unwrap(),
                currency: currency.to_owned(),
                date: None,
                label: None,
            });
            Position::new(Amount::new(number.parse().unwrap(), currency), cost)
        };
        // expected orders observed from beancount's inventory output
        let currencies = ["XYZ", "AB", "ABCD", "CHF", "NZD", "AUD", "GBP", "CAD", "JPY", "EUR", "USD", "CNY", "A"];
        let ordered = beancount_order(currencies.iter().map(|currency| position("1", currency, None)).collect());
        assert_eq!(
            ordered.join(", "),
            "1 USD, 1 EUR, 1 JPY, 1 CAD, 1 GBP, 1 AUD, 1 NZD, 1 CHF, 1 A, 1 AB, 1 XYZ, 1 CNY, 1 ABCD"
        );
        let ordered = beancount_order(vec![
            position("3", "AAA", None),
            position("1", "VHT", Some(("100", "USD"))),
            position("2", "VEA", Some(("50", "USD"))),
            position("4", "GLD", Some(("50", "CAD"))),
            position("1", "ZZZ", None),
        ]);
        assert_eq!(ordered.join(", "), "1 ZZZ, 3 AAA, 4 GLD {50 CAD}, 2 VEA {50 USD}, 1 VHT {100 USD}");
    }

    #[test]
    fn displays_the_period() {
        let period = Period {
            open: Some(PeriodDate::Fixed(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())),
            close: Some(Some(PeriodDate::Param(ParamRef::Named("to".into()), Span::default()))),
            clear: true,
        };
        assert_eq!(period.to_string(), "OPEN ON 2024-01-01 CLOSE ON :to CLEAR");
        let bare = Period {
            open: None,
            close: Some(None),
            clear: false,
        };
        assert_eq!(bare.to_string(), "CLOSE");
    }
}
