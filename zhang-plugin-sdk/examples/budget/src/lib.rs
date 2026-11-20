//! `zhang:budget` as an analyzer, built with `zhang-plugin-sdk`: zhang's budgets (the tables `#budgets`,
//! `#budget_events`, `#budget_definitions`, the `budgets` column of `postings`, and the budget errors) computed from the
//! budget records and queries, as zhang's native budget code computes them.
//!
//! It is the example of an analyzer that does real work, and the host's tests run it in place of the native budget code,
//! as the default plugin `zhang:budget`, to check that an analyzer's diagnostics land at the places and ids the native
//! checks give them and that its tables answer the budget pages alike (RFC #698 P1.4). The official `zhang:budget` is a
//! Lua script (P1.6 on); this one follows the same steps (RFC #698 Appendix A):
//!
//! 1. the records, in the order zhang processed the ledger, with the postings of the budgets' accounts between them: a
//!    `budget` defines a budget (another of its name is `DefineDuplicatedBudget`), an add, transfer or close of an
//!    undefined budget is `BudgetDoesNotExist`, a posting to an account of a budget not defined yet or closed is
//!    `BudgetDoesNotExist` or `BudgetClosed`, once per account and budget;
//! 2. the amounts in another commodity than their budget's that no price converts, `BudgetCommodityMismatch` once
//!    every price is known;
//! 3. the activity of every budget per month: the postings that count toward it, converted by zhang;
//! 4. the facts of `#budgets`, the months with something, with the running figures, which zhang fills between.
//!
//! What crosses the boundary is the budget records, the postings of the budgets' accounts and the conversions: the sums
//! are exact, in [`zhang_plugin_sdk::bigdecimal`], as zhang adds.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;

use zhang_plugin_sdk::analysis::{self, Cause, Conversion, ConvertedAmount, Diagnostic, Input, LabelOutput, Manifest, Output, PartitionOutput, Record, Target};
use zhang_plugin_sdk::ast::Account;
use zhang_plugin_sdk::bigdecimal::{BigDecimal, Zero};
use zhang_plugin_sdk::chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use zhang_plugin_sdk::router::{self, Param, Params, QueryResult};
use zhang_plugin_sdk::serde_json::{self, json, Map, Value};
use zhang_plugin_sdk::{plugin, Error};

plugin! {
    name: "zhang:budget",
    version: env!("CARGO_PKG_VERSION"),
    analyzer: analyze,
    manifest: manifest,
}

/// The manifest: the native provider's tables and column, as zhang describes them, and the budget errors.
pub fn manifest() -> Manifest {
    serde_json::from_str(include_str!("../manifest.json")).expect("the manifest is JSON")
}

/// An amount: its exact number and its commodity.
#[derive(Debug, Clone)]
struct Amount {
    number: BigDecimal,
    currency: String,
}

impl Amount {
    fn of(fields: &Map<String, Value>, key: &str) -> Result<Amount, Error> {
        let amount = &fields[key];
        let number = amount["number"].as_str().ok_or_else(|| Error::msg(format!("{key} has no number")))?;
        Ok(Amount {
            number: decimal(number)?,
            currency: amount["currency"].as_str().unwrap_or_default().to_owned(),
        })
    }

    fn json(&self) -> Value {
        json!({"number": plain(&self.number), "currency": self.currency})
    }
}

fn decimal(text: &str) -> Result<BigDecimal, Error> {
    BigDecimal::from_str(text).map_err(|e| Error::msg(format!("{text} is no decimal: {e}")))
}

fn plain(number: &BigDecimal) -> String {
    zhang_plugin_sdk::decimal::plain_decimal(number)
}

fn date(text: &str) -> Result<NaiveDate, Error> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|e| Error::msg(format!("{text} is no date: {e}")))
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    date.with_day(1).expect("every month has a first day")
}

/// The close of a budget: its date, and its time when written with one.
#[derive(Debug, Clone)]
struct Close {
    date: NaiveDate,
    time: Option<String>,
}

impl Close {
    /// whether the wall-clock time `at` comes after the close: one with a date only lasts through its day
    fn precedes(&self, at: NaiveDateTime) -> bool {
        match &self.time {
            None => at.date() > self.date,
            Some(time) => NaiveTime::parse_from_str(time, "%H:%M:%S").is_ok_and(|time| at > self.date.and_time(time)),
        }
    }
}

/// A budget, as its first `budget` defines it.
struct Budget<'a> {
    record: &'a Record,
    commodity: String,
    close: Option<Close>,
    /// the latest record of it (the later on a tie): its date and seq
    latest: (NaiveDate, u32),
}

impl Budget<'_> {
    fn saw(&mut self, date: NaiveDate, seq: u32) {
        if self.latest.0 <= date {
            self.latest = (date, seq);
        }
    }

    fn meta(&self, key: &str) -> Value {
        self.record
            .meta
            .get(key)
            .and_then(|values| values.first())
            .map_or(Value::Null, |value| json!(value))
    }
}

/// An effect of a budget directive, a row of `#budget_events`.
struct Event<'a> {
    name: String,
    record: &'a Record,
    kind: &'static str,
    amount: Option<Amount>,
}

/// One posting as written: its legs summed, with its first leg's cost currency.
struct Posting {
    seq: u32,
    account: String,
    at: NaiveDateTime,
    units: BigDecimal,
    currency: String,
    via: Option<String>,
    budgets: Vec<String>,
}

/// One booked leg of a posting of a budget's account.
struct Leg {
    seq: u32,
    posting_index: i64,
    account: String,
    date: NaiveDate,
    at: NaiveDateTime,
    number: BigDecimal,
    currency: String,
    cost_currency: Option<String>,
    budgets: Vec<String>,
}

/// An amount of a budget in another commodity, which a price must convert.
struct Foreign {
    seq: u32,
    amount: Amount,
    via: Option<String>,
    date: NaiveDate,
    budget: String,
    budget_commodity: String,
    account: Option<String>,
}

/// The cells of `result` by column name.
fn rows(result: &QueryResult) -> Vec<HashMap<&str, &Value>> {
    result
        .rows
        .iter()
        .map(|row| result.columns.iter().map(|column| column.name.as_str()).zip(row.iter()).collect())
        .collect()
}

fn text(cell: Option<&&Value>) -> String {
    cell.and_then(|it| it.as_str()).unwrap_or_default().to_owned()
}

fn strings(cell: Option<&&Value>) -> Vec<String> {
    cell.and_then(|it| it.as_array())
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The accounts of every budget: those whose `open` names it in its `budget` metadata, at any time.
fn budget_accounts() -> Result<BTreeMap<String, BTreeSet<String>>, Error> {
    let result = router::query("SELECT accounts, meta_values('budget') AS budgets FROM #entries WHERE type = 'open'")?;
    let mut accounts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows(&result) {
        for budget in strings(row.get("budgets")) {
            accounts.entry(budget).or_default().extend(strings(row.get("accounts")));
        }
    }
    Ok(accounts)
}

/// The rows of a page of a query: at most this many, so that a page stays well within the values a query may answer.
const PAGE: usize = 50_000;

/// The booked legs of the postings of `accounts`, in ledger order, with the budgets of their account's `open` in effect
/// at their transaction's date and time: an account at a time, a page at a time.
fn legs(accounts: &BTreeSet<String>) -> Result<Vec<Leg>, Error> {
    let mut legs = vec![];
    for account in accounts {
        for page in 0.. {
            let params = Params::new().bind("account", Param::Str(Some(account.clone())));
            let result = router::query_with_params(
                &format!(
                    "SELECT seq, posting_index, date, time, number, currency, cost_currency, \
                            open_meta_values(account, 'budget', date, time) AS budgets \
                     WHERE account = :account LIMIT {PAGE} OFFSET {}",
                    page * PAGE
                ),
                &params,
            )?;
            let rows = rows(&result);
            let full = rows.len() == PAGE;
            for row in rows {
                let date = date(&text(row.get("date")))?;
                let time = NaiveTime::parse_from_str(&text(row.get("time")), "%H:%M:%S").unwrap_or(NaiveTime::MIN);
                legs.push(Leg {
                    seq: row.get("seq").and_then(|it| it.as_u64()).unwrap_or_default() as u32,
                    posting_index: row.get("posting_index").and_then(|it| it.as_i64()).unwrap_or_default(),
                    account: account.clone(),
                    date,
                    at: date.and_time(time),
                    number: decimal(&text(row.get("number")))?,
                    currency: text(row.get("currency")),
                    cost_currency: row.get("cost_currency").and_then(|it| it.as_str()).map(str::to_owned),
                    budgets: strings(row.get("budgets")),
                });
            }
            if !full {
                break;
            }
        }
    }
    // in ledger order: by transaction, then by leg, which the rows of one account keep
    legs.sort_by_key(|leg| (leg.seq, leg.posting_index));
    Ok(legs)
}

/// The last transaction the load accepted, of the latest date the one folded last: its date and seq.
fn last_transaction() -> Result<Option<(NaiveDate, u32)>, Error> {
    let result = router::query("SELECT seq, date FROM #transactions ORDER BY date DESC, seq DESC LIMIT 1")?;
    let rows = rows(&result);
    let Some(row) = rows.first() else { return Ok(None) };
    Ok(Some((
        date(&text(row.get("date")))?,
        row.get("seq").and_then(|it| it.as_u64()).unwrap_or_default() as u32,
    )))
}

/// `amounts` converted to their targets by zhang: `None` for one no price converts.
fn convert(amounts: &[(Amount, String, NaiveDate, Option<String>)]) -> Result<Vec<Option<BigDecimal>>, Error> {
    let mut converted = Vec::with_capacity(amounts.len());
    for chunk in amounts.chunks(analysis::MAX_CONVERSIONS) {
        let conversions = chunk
            .iter()
            .map(|(amount, target, date, via)| Conversion {
                amount: ConvertedAmount {
                    number: plain(&amount.number),
                    currency: amount.currency.clone(),
                },
                target: target.clone(),
                date: date.format("%Y-%m-%d").to_string(),
                via: via.clone(),
            })
            .collect::<Vec<_>>();
        for answer in analysis::convert(&conversions)? {
            converted.push(answer.map(|amount| decimal(&amount.number)).transpose()?);
        }
    }
    Ok(converted)
}

/// A diagnostic of a legacy budget kind at the entry `seq`.
fn diagnostic(kind: &str, seq: u32, metas: &[(&str, &str)]) -> Diagnostic {
    Diagnostic {
        kind: kind.to_owned(),
        target: Target {
            seq: Some(seq),
            ..Target::default()
        },
        metas: metas.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect(),
        message: None,
    }
}

fn analyze(input: Input) -> Result<Output, Error> {
    let accounts = budget_accounts()?;
    let all_accounts = accounts.values().flatten().cloned().collect::<BTreeSet<_>>();
    let legs = legs(&all_accounts)?;
    let last_transaction = last_transaction()?;

    // the postings as written, by their transaction's seq
    let mut postings: BTreeMap<u32, Vec<Posting>> = BTreeMap::new();
    let mut written: BTreeMap<(u32, i64), usize> = BTreeMap::new();
    for leg in &legs {
        let of_seq = postings.entry(leg.seq).or_default();
        match written.get(&(leg.seq, leg.posting_index)) {
            Some(at) => of_seq[*at].units += &leg.number,
            None => {
                written.insert((leg.seq, leg.posting_index), of_seq.len());
                of_seq.push(Posting {
                    seq: leg.seq,
                    account: leg.account.clone(),
                    at: leg.at,
                    units: leg.number.clone(),
                    currency: leg.currency.clone(),
                    via: leg.cost_currency.clone(),
                    budgets: leg.budgets.clone(),
                });
            }
        }
    }

    // the fold: the records and the transactions in the order zhang processed them
    let mut budgets: BTreeMap<String, Budget<'_>> = BTreeMap::new();
    let mut events: Vec<Event<'_>> = vec![];
    let mut foreign: Vec<Foreign> = vec![];
    let mut diagnostics: Vec<Diagnostic> = vec![];
    let mut reported_undefined: HashSet<(String, String)> = HashSet::new();
    let mut reported_closed: HashSet<(String, String)> = HashSet::new();
    let keep_foreign = |budgets: &BTreeMap<String, Budget<'_>>,
                        foreign: &mut Vec<Foreign>,
                        name: &str,
                        amount: &Amount,
                        via: Option<String>,
                        date,
                        seq,
                        account: Option<String>| {
        let Some(budget) = budgets.get(name) else { return };
        if amount.currency == budget.commodity || amount.number.is_zero() {
            return;
        }
        foreign.push(Foreign {
            seq,
            amount: amount.clone(),
            via,
            date,
            budget: name.to_owned(),
            budget_commodity: budget.commodity.clone(),
            account,
        });
    };
    let mut records = input.records.iter().peekable();
    let mut transactions = postings.iter().peekable();
    loop {
        let record_first = match (records.peek(), transactions.peek()) {
            (None, None) => break,
            (Some(record), Some((seq, _))) => record.seq < **seq,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        if !record_first {
            let (_, of_transaction) = transactions.next().expect("peeked");
            for posting in of_transaction {
                let mut names = posting.budgets.clone();
                names.sort();
                for name in names {
                    let key = (posting.account.clone(), name.clone());
                    let Some(budget) = budgets.get(&name) else {
                        if reported_undefined.insert(key) {
                            diagnostics.push(diagnostic(
                                "BudgetDoesNotExist",
                                posting.seq,
                                &[("account_name", &posting.account), ("budget_name", &name)],
                            ));
                        }
                        continue;
                    };
                    if budget.close.as_ref().is_some_and(|close| close.precedes(posting.at)) {
                        if reported_closed.insert(key) {
                            diagnostics.push(diagnostic(
                                "BudgetClosed",
                                posting.seq,
                                &[("account_name", &posting.account), ("budget_name", &name)],
                            ));
                        }
                        continue;
                    }
                    let units = Amount {
                        number: posting.units.clone(),
                        currency: posting.currency.clone(),
                    };
                    keep_foreign(
                        &budgets,
                        &mut foreign,
                        &name,
                        &units,
                        posting.via.clone(),
                        posting.at.date(),
                        posting.seq,
                        Some(posting.account.clone()),
                    );
                }
            }
            continue;
        }
        let record = records.next().expect("peeked");
        let fields = &record.fields;
        let field = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
        let on = date(&record.date)?;
        match record.kind.as_str() {
            "budget" => {
                let name = field("name");
                let commodity = field("commodity");
                if let Some(defined) = budgets.get(&name) {
                    diagnostics.push(diagnostic(
                        "DefineDuplicatedBudget",
                        record.seq,
                        &[("budget_name", &name), ("budget_commodity", &defined.commodity), ("commodity", &commodity)],
                    ));
                    continue;
                }
                budgets.insert(
                    name,
                    Budget {
                        record,
                        commodity,
                        close: None,
                        latest: (on, record.seq),
                    },
                );
            }
            "budget-add" => {
                let name = field("name");
                if !budgets.contains_key(&name) {
                    diagnostics.push(diagnostic("BudgetDoesNotExist", record.seq, &[("budget_name", &name)]));
                    continue;
                }
                let amount = Amount::of(fields, "amount")?;
                keep_foreign(&budgets, &mut foreign, &name, &amount, None, on, record.seq, None);
                budgets.get_mut(&name).expect("defined").saw(on, record.seq);
                events.push(Event {
                    name,
                    record,
                    kind: "assign",
                    amount: Some(amount),
                });
            }
            "budget-transfer" => {
                let (from, to) = (field("from"), field("to"));
                // `from`, then `to`, as the native check reports them
                if let Some(undefined) = [&from, &to].into_iter().find(|name| !budgets.contains_key(*name)) {
                    diagnostics.push(diagnostic("BudgetDoesNotExist", record.seq, &[("budget_name", undefined)]));
                    continue;
                }
                let amount = Amount::of(fields, "amount")?;
                for name in [&from, &to] {
                    keep_foreign(&budgets, &mut foreign, name, &amount, None, on, record.seq, None);
                }
                let out = Amount {
                    number: -amount.number.clone(),
                    currency: amount.currency.clone(),
                };
                for (name, kind, amount) in [(from, "transfer_out", out), (to, "transfer_in", amount)] {
                    budgets.get_mut(&name).expect("defined").saw(on, record.seq);
                    events.push(Event {
                        name,
                        record,
                        kind,
                        amount: Some(amount),
                    });
                }
            }
            "budget-close" => {
                let name = field("name");
                let Some(budget) = budgets.get_mut(&name) else {
                    diagnostics.push(diagnostic("BudgetDoesNotExist", record.seq, &[("budget_name", &name)]));
                    continue;
                };
                budget.saw(on, record.seq);
                // a budget closes with its first budget-close
                budget.close.get_or_insert(Close {
                    date: on,
                    time: record.has_time.then(|| record.time.clone()),
                });
                events.push(Event {
                    name,
                    record,
                    kind: "close",
                    amount: None,
                });
            }
            _ => {}
        }
    }

    // the amounts no price converts, once every price is known, in the order the fold kept them
    let checked = convert(
        &foreign
            .iter()
            .map(|it| (it.amount.clone(), it.budget_commodity.clone(), it.date, it.via.clone()))
            .collect::<Vec<_>>(),
    )?;
    for (amount, converted) in foreign.iter().zip(checked) {
        if converted.is_some() {
            continue;
        }
        let mut metas = vec![
            ("budget_name", amount.budget.as_str()),
            ("commodity", amount.amount.currency.as_str()),
            ("budget_commodity", amount.budget_commodity.as_str()),
        ];
        if let Some(account) = &amount.account {
            metas.push(("account_name", account));
        }
        diagnostics.push(diagnostic("BudgetCommodityMismatch", amount.seq, &metas));
    }

    // what the directives put in, converted to the budget's commodity at their date, per budget and month
    let mut added: BTreeMap<(String, NaiveDate), BigDecimal> = BTreeMap::new();
    let mut to_convert = vec![];
    let mut converting = vec![];
    for event in &events {
        let (Some(budget), Some(amount)) = (budgets.get(&event.name), &event.amount) else {
            continue;
        };
        let on = date(&event.record.date)?;
        let key = (event.name.clone(), first_of_month(on));
        if amount.currency == budget.commodity {
            *added.entry(key).or_insert_with(BigDecimal::zero) += &amount.number;
        } else {
            to_convert.push((amount.clone(), budget.commodity.clone(), on, None));
            converting.push(key);
        }
    }
    for (key, converted) in converting.into_iter().zip(convert(&to_convert)?) {
        if let Some(number) = converted {
            *added.entry(key).or_insert_with(BigDecimal::zero) += number;
        }
    }

    // the activity of every budget per month: the legs that count toward it, converted, negated on an account of a
    // negative type
    let mut activity: BTreeMap<(String, NaiveDate), BigDecimal> = BTreeMap::new();
    let mut to_convert = vec![];
    let mut converting = vec![];
    for leg in &legs {
        let negated = Account::from_str(&leg.account).is_ok_and(|account| account.get_account_sign() < 0);
        for name in &leg.budgets {
            let Some(budget) = budgets.get(name) else { continue };
            let counts = accounts.get(name).is_some_and(|it| it.contains(&leg.account))
                && leg.seq > budget.record.seq
                && !budget.close.as_ref().is_some_and(|close| close.precedes(leg.at));
            if !counts {
                continue;
            }
            let key = (name.clone(), first_of_month(leg.date));
            if leg.currency == budget.commodity {
                let entry = activity.entry(key).or_insert_with(BigDecimal::zero);
                if negated {
                    *entry -= &leg.number;
                } else {
                    *entry += &leg.number;
                }
            } else {
                let units = Amount {
                    number: leg.number.clone(),
                    currency: leg.currency.clone(),
                };
                to_convert.push((units, budget.commodity.clone(), leg.date, leg.cost_currency.clone()));
                converting.push((key, negated));
            }
        }
    }
    for ((key, negated), converted) in converting.into_iter().zip(convert(&to_convert)?) {
        if let Some(number) = converted {
            let entry = activity.entry(key).or_insert_with(BigDecimal::zero);
            if negated {
                *entry -= number;
            } else {
                *entry += number;
            }
        }
    }

    let mut output = Output {
        diagnostics,
        ..Output::default()
    };
    let meta = |record: &Record| json!(record.meta);
    let accounts_of = |name: &str| json!(accounts.get(name).cloned().unwrap_or_default());

    // #budget_definitions and the labels of postings.budgets, by name
    let mut definitions = vec![];
    let mut labels = vec![];
    for (name, budget) in &budgets {
        let close_time = budget.close.as_ref().and_then(|close| close.time.clone());
        definitions.push(object(json!({
            "name": name, "date": budget.record.date, "currency": budget.commodity, "alias": budget.meta("alias"),
            "category": budget.meta("category"), "accounts": accounts_of(name),
            "close": budget.close.as_ref().map(|close| close.date.format("%Y-%m-%d").to_string()), "close_time": close_time,
            "$meta": meta(budget.record),
        })));
        labels.push(LabelOutput {
            label: name.clone(),
            params: object(json!({
                "accounts": accounts_of(name), "defined_seq": budget.record.seq,
                "close_date": budget.close.as_ref().map(|close| close.date.format("%Y-%m-%d").to_string()),
                "close_time": close_time, "commodity": budget.commodity,
            })),
        });
    }
    output.tables.insert("budget_definitions".to_owned(), definitions);
    output.labels.insert("budgets".to_owned(), labels);

    // #budget_events, in ledger order
    let rows = events
        .iter()
        .map(|event| {
            object(json!({
                "name": event.name, "date": event.record.date, "time": event.record.time, "timestamp": event.record.timestamp,
                "type": event.kind, "amount": event.amount.as_ref().map(Amount::json), "$meta": meta(event.record),
            }))
        })
        .collect();
    output.tables.insert("budget_events".to_owned(), rows);

    // #budgets: the months with something of every budget, by name, with the running figures
    let mut partitions = vec![];
    for (name, budget) in &budgets {
        let first = first_of_month(date(&budget.record.date)?);
        let close_month = budget.close.as_ref().map(|close| first_of_month(close.date));
        let mut months = added
            .keys()
            .chain(activity.keys())
            .filter(|(budget, _)| budget == name)
            .map(|(_, month)| *month)
            .filter(|month| *month >= first)
            .collect::<BTreeSet<_>>();
        months.insert(first);
        months.extend(close_month.filter(|month| *month >= first));
        let amount = |number: &BigDecimal| json!({"number": plain(number), "currency": budget.commodity});
        let mut available = BigDecimal::zero();
        let mut rows = vec![];
        for month in months {
            let key = (name.clone(), month);
            let added = added.get(&key).cloned().unwrap_or_else(BigDecimal::zero);
            let activity = activity.get(&key).cloned().unwrap_or_else(BigDecimal::zero);
            let assigned = &available + &added;
            available = &assigned - &activity;
            rows.push(object(json!({
                "alias": budget.meta("alias"), "category": budget.meta("category"), "currency": budget.commodity,
                "date": month.format("%Y-%m-%d").to_string(),
                "assigned": amount(&assigned), "added": amount(&added), "activity": amount(&activity), "available": amount(&available),
                "accounts": accounts_of(name), "closed": close_month.is_some_and(|close| close <= month),
                "$meta": meta(budget.record),
            })));
        }
        // its own last month, or the last month with a transaction when that is later
        let mut until = (first_of_month(budget.latest.0), budget.latest.1);
        if let Some((date, seq)) = last_transaction {
            if first_of_month(date) > until.0 {
                until = (first_of_month(date), seq);
            }
        }
        partitions.push(PartitionOutput {
            key: object(json!({ "name": name })),
            until: until.0.format("%Y-%m-%d").to_string(),
            until_cause: Some(Cause { seq: until.1 }),
            first_cause: Some(Cause { seq: budget.record.seq }),
            rows,
        });
    }
    output.series.insert("budgets".to_owned(), analysis::SeriesOutput { partitions });
    Ok(output)
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}
