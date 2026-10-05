//! Built-in queries: the named, documented BQL behind the figures the app shows (#479).
//!
//! A read endpoint computes its figures with one or more of these queries and only maps the
//! result into its response, so the business logic lives in the query engine and every figure
//! can be opened, and adapted, on the Query page (`/explore`): `GET /api/query/builtins` lists the
//! queries, `POST /api/query/builtins/{name}/text` writes one out with its parameters filled
//! in, and the "Built-in queries" page of the docs lists them with their BQL.
//!
//! # Adding a query
//!
//! 1. Add a [`BuiltinQuery`] to [`BUILTINS`], in the section of the endpoints that run it: a
//!    unique dotted lower-case name (`report.summary`), a one-sentence description, the BQL,
//!    and every parameter it uses with its type. Parameters are named (`:from`), and their
//!    types are those a JSON value can give (see [`json_params`]).
//! 2. Run it with [`run`], or with [`execute`] inside [`crate::routes::query::with_ledger`] to
//!    run several under one read lock. User input is only ever bound as a parameter, never
//!    formatted into the BQL.
//! 3. Map the [`QueryResult`] into the endpoint's response; [`calculated_amount`] and
//!    [`LedgerDateRange`] are the shared pieces of that mapping.
//! 4. List the query, with its BQL as it is here, on the "Built-in queries" page of the docs
//!    (`docs/src/content/docs/reference/builtin-queries.md` and its `zh-cn` twin).
//!
//! The tests check that every query compiles, declares exactly the parameters its BQL uses,
//! can be written out as BQL and is documented.

mod amount;
mod date_range;

use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::LazyLock;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_core::ledger::Ledger;
use zhang_query::{DataType, ExecuteOptions, ParamTypes, Params, Query, QueryResult, Value};

pub use self::amount::calculated_amount;
pub use self::date_range::{ledger_date, LedgerDateRange};
use crate::error::ServerError;
use crate::request::BuiltinParamValue;
use crate::routes::query::{execute_options, max_result_values, with_ledger};
use crate::{LedgerState, ServerResult};

/// A named BQL query the app runs to compute what it shows.
#[derive(Debug)]
pub struct BuiltinQuery {
    /// unique, dotted and lower case, e.g. `report.summary`
    pub name: &'static str,
    /// one sentence on what the query returns
    pub description: &'static str,
    /// the query, with its parameters written `:name`
    pub bql: &'static str,
    /// every parameter the query uses, with its type
    pub params: &'static [(&'static str, DataType)],
}

impl BuiltinQuery {
    /// The declared types of the parameters, to compile the query with.
    pub fn param_types(&self) -> ParamTypes {
        self.params.iter().fold(ParamTypes::new(), |types, (name, ty)| types.bind(*name, *ty))
    }
}

/// Every built-in query, grouped by the endpoints that run it.
pub static BUILTINS: &[BuiltinQuery] = &[
    // ---- general ----
    BuiltinQuery {
        name: "postings.between",
        description: "Every posting between two ledger dates, both included, in ledger order.",
        bql: "SELECT date, flag, payee, narration, account, position \
              WHERE date >= :from AND date <= :to \
              ORDER BY seq",
        params: &[("from", DataType::Date), ("to", DataType::Date)],
    },
    BuiltinQuery {
        name: "postings.matching",
        description: "The postings of the transactions with a payee and any of some tags, in ledger order; a NULL parameter leaves its filter out.",
        bql: "SELECT date, payee, narration, tags, account, position \
              WHERE (:payee IS NULL OR payee = :payee) AND (:tags IS NULL OR intersects(tags, :tags)) \
              ORDER BY seq",
        params: &[("payee", DataType::Str), ("tags", DataType::Set)],
    },
    // ---- report: /api/statistic/* ----
    crate::report::NET_WORTH,
    crate::report::LIABILITIES,
    crate::report::FLOWS,
    crate::report::TRANSACTION_COUNT,
    crate::report::NET_WORTH_TREND,
    crate::report::CHANGES,
    crate::report::ACCOUNT_TOTALS,
    crate::report::TOP_POSTINGS,
    // ---- accounts: /api/accounts/* ----
    // an account's page is its subtree: `under(account, :account)`
    BuiltinQuery {
        name: "accounts.list",
        description: "Every account with an open or close directive, with its open and close dates, its alias and its status today, \
                      by name.",
        bql: "SELECT account, open, close, meta('alias') AS alias, account_status(account, today()) AS status
FROM #accounts
ORDER BY account",
        params: &[],
    },
    BuiltinQuery {
        name: "accounts.balances",
        description: "The balance of every account of its own postings per currency: the units, their value in the operating currency \
                      at today's prices, and the date of the first posting.",
        bql: "SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency",
        params: &[("operating_currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.subtree",
        description: "An account and those of its sub-accounts that have an open or close directive, with their open and close dates, \
                      their aliases and their statuses today, by name.",
        bql: "SELECT account, open, close, meta('alias') AS alias, account_status(account, today()) AS status
FROM #accounts
WHERE under(account, :account)
ORDER BY account",
        params: &[("account", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.subtree_balances",
        description: "The balance of an account and of each of its sub-accounts of their own postings per currency: the units, their \
                      value in the operating currency at today's prices, and the date of the first posting.",
        bql: "SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency",
        params: &[("account", DataType::Str), ("operating_currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.journal",
        description: "The postings of an account and its sub-accounts in ledger order, a row per posting, the lots it is booked \
                      against added up, each with the running balance of the account and its sub-accounts right after it.",
        bql: "SELECT first(date) AS date, first(time) AS time, first(timestamp) AS timestamp, first(flag) AS flag,
       first(id) AS id, first(account) AS account, first(payee) AS payee, first(narration) AS narration,
       seq, posting_index, sum(number) AS units, first(currency) AS currency, last(units(balance)) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index",
        params: &[("account", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.journal_page",
        description: "Some rows of accounts.journal: from an offset, at most a limit of them.",
        bql: "SELECT first(date) AS date, first(time) AS time, first(timestamp) AS timestamp, first(flag) AS flag,
       first(id) AS id, first(account) AS account, first(payee) AS payee, first(narration) AS narration,
       seq, posting_index, sum(number) AS units, first(currency) AS currency, last(units(balance)) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index
LIMIT :limit OFFSET :offset",
        params: &[("account", DataType::Str), ("limit", DataType::Int), ("offset", DataType::Int)],
    },
    BuiltinQuery {
        name: "accounts.balance_assertions",
        description: "The balance assertions on an account, newest first, with the balance of the account and its sub-accounts each \
                      was checked against, whether it held, and the account a balance with pad pads from.",
        bql: "SELECT date, time, timestamp, id, account, amount, actual, passed, pad, seq
FROM #balances
WHERE account = :account
ORDER BY seq DESC",
        params: &[("account", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.balance_history",
        description: "The balance of an account and its sub-accounts at the end of every day with a posting, per currency, in date order.",
        bql: "SELECT date, currency, last(only(currency, units(balance))) AS balance
WHERE under(account, :account)
GROUP BY date, currency
ORDER BY date, currency",
        params: &[("account", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.documents",
        description: "The document directives of an account and its sub-accounts, in ledger order, with the path of each file relative to \
                      the ledger's directory.",
        bql: "SELECT date, time, account, path, transaction_id
FROM #documents
WHERE source = 'directive' AND under(account, :account)",
        params: &[("account", DataType::Str)],
    },
    // ---- journals: /api/journals, /api/for-new-transaction, /api/documents, /api/errors ----
    BuiltinQuery {
        name: "journals.page",
        description: "One page of the journal, newest first: the transactions, padding transactions included, and the balance \
                      assertions that match a keyword, tags and links, where a NULL parameter leaves its filter out.",
        bql: "SELECT seq, type, id, date, time, flag, payee, narration, tags, links, metas \
              FROM #entries \
              WHERE (type = 'transaction' \
                     AND (:tags IS NULL OR intersects(tags, :tags)) \
                     AND (:links IS NULL OR intersects(links, :links)) \
                     AND (:keyword IS NULL OR icontains(payee, :keyword) OR icontains(narration, :keyword) \
                          OR any_icontains(tags, :keyword) OR any_icontains(links, :keyword) OR any_icontains(accounts, :keyword))) \
                 OR (type = 'balance' AND :tags IS NULL AND :links IS NULL \
                     AND (:keyword IS NULL OR icontains('Balance Check', :keyword) OR any_icontains(accounts, :keyword))) \
              ORDER BY seq DESC \
              LIMIT :size OFFSET :offset",
        params: &[
            ("keyword", DataType::Str),
            ("tags", DataType::Set),
            ("links", DataType::Set),
            ("size", DataType::Int),
            ("offset", DataType::Int),
        ],
    },
    BuiltinQuery {
        name: "journals.postings",
        description: "The postings of some transactions as written, in ledger order, with their units, whether those were inferred, \
                      the per-unit costs of their lots and the balance of their account in their currency before and after them.",
        bql: "SELECT id, posting_index, account, automatic, balanced, \
                     first(currency) AS currency, \
                     sum(number) AS number, \
                     count(*) AS lots, count(cost_number) AS lots_at_cost, \
                     min(cost_number) AS cost_number, max(cost_number) AS max_cost_number, \
                     min(cost_currency) AS cost_currency, max(cost_currency) AS max_cost_currency, \
                     number(last(only(currency, account_balance))) - sum(number) AS balance_before, \
                     number(last(only(currency, account_balance))) AS balance_after, \
                     first(metas) AS metas \
              WHERE id IN :ids \
              GROUP BY id, posting_index, account, automatic, balanced",
        params: &[("ids", DataType::Set)],
    },
    BuiltinQuery {
        name: "journals.balance_checks",
        description: "Some balance assertions with the asserted amount, the account's true balance, their difference and whether the \
                      assertion holds.",
        bql: "SELECT id, account, amount, tolerance, actual, passed, \
                     amount - actual AS difference, \
                     actual + (amount - actual) AS asserted \
              FROM #balances \
              WHERE id IN :ids",
        params: &[("ids", DataType::Set)],
    },
    BuiltinQuery {
        name: "journals.payees",
        description: "Every payee of the ledger's transactions, once and sorted, without those of the padding transactions.",
        bql: "SELECT DISTINCT payee \
              FROM #transactions \
              WHERE payee IS NOT NULL AND payee != '' AND flag != 'P' \
              ORDER BY payee",
        params: &[],
    },
    BuiltinQuery {
        name: "journals.accounts",
        description: "The accounts open today, sorted by name.",
        bql: "SELECT account \
              FROM #accounts \
              WHERE account_status(account, today()) = 'open' \
              ORDER BY account",
        params: &[],
    },
    BuiltinQuery {
        name: "journals.documents",
        description: "Every document of the ledger, newest first: the document directives and the documents that transactions and \
                      their postings name in their metadata.",
        bql: "SELECT date, time, path, account, transaction_id \
              FROM #documents \
              ORDER BY seq DESC",
        params: &[],
    },
    BuiltinQuery {
        name: "journals.errors",
        description: "One page of the ledger's errors, by file and then by position in the file.",
        bql: "SELECT id, kind, file, line, column, span_start, span_end, source, metas \
              FROM #errors \
              LIMIT :size OFFSET :offset",
        params: &[("size", DataType::Int), ("offset", DataType::Int)],
    },
    // ---- budgets and commodities: /api/budgets/*, /api/commodities/* ----
    BuiltinQuery {
        name: "budgets.month",
        description: "Every budget as of a month (its first day): its last month in #budgets up to that month, carried over to the month when it is later.",
        bql: "SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, \
              last(date) AS last_month, \
              CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned, \
              CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity, \
              last(available) AS available, last(closed) AS closed \
              FROM #budgets \
              WHERE date <= :month \
              GROUP BY name \
              ORDER BY name",
        params: &[("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "budgets.budget",
        description: "One budget: its display name, category, commodity, the accounts whose postings are its activity, and the date and time of its close.",
        bql: "SELECT name, alias, category, currency, accounts, close, close_time \
              FROM #budget_definitions \
              WHERE name = :name",
        params: &[("name", DataType::Str)],
    },
    BuiltinQuery {
        name: "budgets.budget_month",
        description: "One budget as of a month (its first day), as in budgets.month.",
        bql: "SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, \
              last(date) AS last_month, \
              CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned, \
              CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity, \
              last(available) AS available, last(closed) AS closed \
              FROM #budgets \
              WHERE name = :name AND date <= :month \
              GROUP BY name",
        params: &[("name", DataType::Str), ("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "budgets.events",
        description: "What the budget-add and budget-transfer directives put into a budget in a month (its first day), newest first.",
        bql: "SELECT date, time, timestamp, type, amount \
              FROM #budget_events \
              WHERE name = :name AND type != 'close' AND yearmonth(date) = :month \
              ORDER BY timestamp DESC",
        params: &[("name", DataType::Str), ("month", DataType::Date)],
    },
    BuiltinQuery {
        name: "budgets.postings",
        description: "The postings of a budget in a month (its first day), newest first, with each account's balance after them; none after the budget's close (its date, and its time if any).",
        bql: "SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units, \
              only(currency, account_balance) AS balance \
              WHERE account IN :accounts AND yearmonth(date) = :month AND :name IN account_budgets(account, date) \
              AND (:close IS NULL OR date < :close OR (date = :close AND (:close_time IS NULL OR time <= :close_time))) \
              ORDER BY timestamp DESC",
        params: &[
            ("accounts", DataType::Set),
            ("month", DataType::Date),
            ("name", DataType::Str),
            ("close", DataType::Date),
            ("close_time", DataType::Str),
        ],
    },
    BuiltinQuery {
        name: "commodities.totals",
        description: "How many units of each commodity the Assets and Liabilities accounts hold, for the commodities they hold.",
        bql: "SELECT currency, sum(number) AS total \
              WHERE under(account, 'Assets') OR under(account, 'Liabilities') \
              GROUP BY currency \
              HAVING sum(number) != 0 \
              ORDER BY currency",
        params: &[],
    },
    BuiltinQuery {
        name: "commodities.total",
        description: "How many units of a commodity the Assets and Liabilities accounts hold; no row if they hold none.",
        bql: "SELECT currency, sum(number) AS total \
              WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities')) \
              GROUP BY currency \
              HAVING sum(number) != 0",
        params: &[("commodity", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.latest_prices",
        description: "The latest price of each commodity in a currency as of today, with its date and time; the rate is the one the valuations use (the latest quote of the pair on or before today, in either direction, inverted when it is quoted the other way round).",
        bql: "SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency, \
              last(date) AS date, last(time) AS time, \
              getprice(last(CASE WHEN currency = :currency THEN currency(amount) ELSE currency END), :currency, today()) AS rate \
              FROM #prices \
              WHERE (currency = :currency OR currency(amount) = :currency) AND currency != currency(amount) AND date <= today() \
              GROUP BY 1 \
              ORDER BY 1",
        params: &[("currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.latest_price",
        description: "The latest price of a commodity in a currency as of today, with its date and time; the rate is the one the valuations use (the latest quote of the pair on or before today, in either direction, inverted when it is quoted the other way round).",
        bql: "SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency, \
              last(date) AS date, last(time) AS time, getprice(:commodity, :currency, today()) AS rate \
              FROM #prices \
              WHERE ((currency = :commodity AND currency(amount) = :currency) OR (currency = :currency AND currency(amount) = :commodity)) \
              AND currency != currency(amount) AND date <= today() \
              GROUP BY 1",
        params: &[("commodity", DataType::Str), ("currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.lots",
        description:
            "The lots of a commodity the Assets and Liabilities accounts hold, by account, then oldest first; lots differing only by label are kept apart.",
        bql: "SELECT account, cost_date, cost_number, cost_currency, cost_label, sum(number) AS units \
              WHERE currency = :commodity AND (under(account, 'Assets') OR under(account, 'Liabilities')) \
              GROUP BY account, cost_date, cost_number, cost_currency, cost_label \
              HAVING sum(number) != 0 \
              ORDER BY account, cost_date, cost_number, cost_label",
        params: &[("commodity", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.prices",
        description: "Every price of a commodity, in any currency, oldest first.",
        bql: "SELECT date, time, amount \
              FROM #prices \
              WHERE currency = :commodity \
              ORDER BY date, time",
        params: &[("commodity", DataType::Str)],
    },
];

/// The built-in query named `name`.
pub fn get(name: &str) -> Option<&'static BuiltinQuery> {
    BUILTINS.iter().find(|query| query.name == name)
}

/// Every built-in query, compiled once, on first use.
static COMPILED: LazyLock<HashMap<&'static str, Query>> = LazyLock::new(|| {
    BUILTINS
        .iter()
        .map(|builtin| {
            // the tests compile every built-in query, so this cannot fail in a release
            let query = Query::compile_with_params(builtin.bql, &builtin.param_types())
                .unwrap_or_else(|err| panic!("the built-in query {} does not compile: {}", builtin.name, err));
            (builtin.name, query)
        })
        .collect()
});

/// The compiled built-in query named `name`.
pub fn compiled(name: &str) -> ServerResult<&'static Query> {
    COMPILED.get(name).ok_or_else(|| ServerError::UnknownBuiltinQuery(name.to_owned()))
}

/// Execute the built-in query `name` on `ledger`, with the limits of every query the server
/// runs (the time limit and the result size limit of `POST /api/query`). With `count_total`
/// the result's `total` is the number of rows before `LIMIT` and `OFFSET`.
///
/// This blocks: call it inside [`with_ledger`], which also runs several queries under one read
/// lock, or use [`run`].
pub fn execute(ledger: &Ledger, name: &str, params: &Params, count_total: bool) -> ServerResult<QueryResult> {
    let options = ExecuteOptions {
        count_total,
        ..execute_options(max_result_values())
    };
    Ok(compiled(name)?.execute_with_options(ledger, params, &options)?)
}

/// Run the built-in query `name` on the ledger, off the async workers and under its read lock,
/// as `POST /api/query` runs a query.
pub async fn run(ledger: &LedgerState, name: &'static str, params: Params) -> ServerResult<QueryResult> {
    with_ledger(ledger, move |ledger| execute(ledger, name, &params, false)).await
}

/// [`run`], also counting the rows before `LIMIT` and `OFFSET` into the result's `total`.
pub async fn run_with_total(ledger: &LedgerState, name: &'static str, params: Params) -> ServerResult<QueryResult> {
    with_ledger(ledger, move |ledger| execute(ledger, name, &params, true)).await
}

/// The parameters of `builtin` from JSON values: every declared parameter must be given, and
/// no other. `null` is NULL for any type; otherwise a `bool` takes a boolean, an `int` an
/// integer, a `decimal` a number or a string such as `"12.50"`, a `str` a string, a `date` a
/// string `YYYY-MM-DD`, and a `set` a list of strings.
pub fn json_params(builtin: &BuiltinQuery, mut values: HashMap<String, Option<BuiltinParamValue>>) -> ServerResult<Params> {
    let mut params = Params::new();
    for (name, ty) in builtin.params {
        let value = values
            .remove(*name)
            .ok_or_else(|| ServerError::InvalidInput(format!("parameter :{} of {} is missing", name, builtin.name)))?;
        let value = json_value(*ty, value).map_err(|expected| ServerError::InvalidInput(format!("parameter :{} must be {}", name, expected)))?;
        params = params.bind(*name, value);
    }
    match values.keys().collect::<BTreeSet<_>>().first() {
        Some(unknown) => Err(ServerError::InvalidInput(format!("{} has no parameter :{}", builtin.name, unknown))),
        None => Ok(params),
    }
}

/// A parameter's value of type `ty` from JSON, or what it expects.
fn json_value(ty: DataType, value: Option<BuiltinParamValue>) -> Result<Value, &'static str> {
    use BuiltinParamValue::*;
    let Some(value) = value else {
        return Ok(Value::Null);
    };
    let decimal = |text: &str| BigDecimal::from_str(text.trim()).map(Value::Decimal).ok();
    let parsed = match (ty, value) {
        (DataType::Bool, Bool(it)) => Some(Value::Bool(it)),
        (DataType::Int, Int(it)) => Some(Value::Int(it)),
        (DataType::Decimal, Int(it)) => Some(Value::Decimal(it.into())),
        // the shortest text that reads back as the same double, the number as it was written
        (DataType::Decimal, Number(it)) => decimal(&it.to_string()),
        (DataType::Decimal, Text(it)) => decimal(&it),
        (DataType::Str, Text(it)) => Some(Value::Str(it)),
        (DataType::Date, Text(it)) => NaiveDate::parse_from_str(&it, "%Y-%m-%d").ok().map(Value::Date),
        (DataType::Set, List(items)) => Some(Value::Set(items.into_iter().collect())),
        _ => None,
    };
    parsed.ok_or(match ty {
        DataType::Bool => "a boolean or null",
        DataType::Int => "an integer or null",
        DataType::Decimal => "a number, a decimal string such as \"12.50\", or null",
        DataType::Str => "a string or null",
        DataType::Date => "a date string YYYY-MM-DD or null",
        DataType::Set => "a list of strings or null",
        _ => "of a type a JSON value cannot give",
    })
}

/// The BQL of `builtin` with `params` written in as literals: a query that runs on the Query
/// page (`/explore`), where parameters cannot be bound, to the same result. Display only; the
/// app itself always binds them.
pub fn text(builtin: &BuiltinQuery, params: &Params) -> ServerResult<String> {
    Ok(compiled(builtin.name)?.inline_params(params)?)
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;

    use zhang_query::{DataType, ParamRef, Query, Value};

    use super::{compiled, json_params, json_value, text, BuiltinQuery, BUILTINS};
    use crate::request::BuiltinParamValue;

    /// The types a parameter of a built-in query can have: those a JSON value can give.
    const JSON_TYPES: [DataType; 6] = [DataType::Bool, DataType::Int, DataType::Decimal, DataType::Str, DataType::Date, DataType::Set];

    /// A value of each JSON type, to write every built-in query out with.
    fn sample(ty: DataType) -> BuiltinParamValue {
        match ty {
            DataType::Bool => BuiltinParamValue::Bool(true),
            DataType::Int => BuiltinParamValue::Int(3),
            DataType::Decimal => BuiltinParamValue::Text("-12.50".to_owned()),
            DataType::Str => BuiltinParamValue::Text(r#"it's "quoted" \ "#.to_owned()),
            DataType::Date => BuiltinParamValue::Text("2024-02-29".to_owned()),
            DataType::Set => BuiltinParamValue::List(vec!["a'b".to_owned(), "c".to_owned()]),
            other => panic!("no JSON sample of {}", other),
        }
    }

    #[test]
    fn every_builtin_compiles_with_exactly_its_declared_params() {
        for builtin in BUILTINS {
            let query = Query::compile_with_params(builtin.bql, &builtin.param_types()).unwrap_or_else(|err| panic!("{}: {}", builtin.name, err));
            let used = query
                .params()
                .into_iter()
                .map(|(param, ty)| match param {
                    ParamRef::Named(name) => (name, ty),
                    ParamRef::Positional(_) => panic!("{} uses {}: name the parameters of a built-in query", builtin.name, param),
                })
                .collect::<BTreeSet<_>>();
            let declared = builtin.params.iter().map(|(name, ty)| (name.to_string(), *ty)).collect::<BTreeSet<_>>();
            assert_eq!(declared.len(), builtin.params.len(), "{} declares a parameter twice", builtin.name);
            assert_eq!(used, declared, "{}: the parameters its BQL uses, and those it declares", builtin.name);
            // the registry compiles the same query
            assert_eq!(compiled(builtin.name).unwrap().source(), builtin.bql);
        }
    }

    #[test]
    fn builtin_names_are_unique_dotted_and_lower_case_and_described() {
        let mut names = BTreeSet::new();
        for BuiltinQuery { name, description, .. } in BUILTINS {
            assert!(names.insert(*name), "{} is registered twice", name);
            let words_ok = name.split('.').all(|word| {
                word.starts_with(|c: char| c.is_ascii_lowercase()) && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            });
            assert!(words_ok, "{} must be dotted lower-case words, such as report.summary", name);
            assert!(
                description.ends_with('.') && !description.contains('\n'),
                "{} needs a one-sentence description",
                name
            );
        }
    }

    #[test]
    fn every_builtin_takes_json_params_and_can_be_written_as_bql() {
        for builtin in BUILTINS {
            for (name, ty) in builtin.params {
                assert!(
                    JSON_TYPES.contains(ty),
                    "{}: parameter :{} is a {}, which a JSON value cannot give",
                    builtin.name,
                    name,
                    ty
                );
            }
            // the counts of LIMIT and OFFSET and the dates of OPEN ON and CLOSE ON take no NULL:
            // the engine rejects one at the parameter, so they keep a value
            let words = builtin.bql.split_whitespace().collect::<Vec<_>>();
            let required = |name: &str| {
                words
                    .windows(2)
                    .any(|it| ["LIMIT", "OFFSET", "ON"].contains(&it[0].to_uppercase().as_str()) && it[1] == format!(":{}", name))
            };
            for null in [false, true] {
                let values = builtin
                    .params
                    .iter()
                    .map(|(name, ty)| (name.to_string(), (!null || required(name)).then(|| sample(*ty))))
                    .collect::<HashMap<_, _>>();
                let params = json_params(builtin, values).unwrap();
                let written = text(builtin, &params).unwrap_or_else(|err| panic!("{}: {}", builtin.name, err));
                Query::compile(&written).unwrap_or_else(|err| panic!("{} written as {}: {}", builtin.name, written, err));
            }
        }
    }

    /// Whitespace runs as one space, so the docs may wrap a query differently.
    fn normalize(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn every_builtin_is_documented_with_its_bql() {
        let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/src/content/docs");
        for file in ["reference/builtin-queries.md", "zh-cn/reference/builtin-queries.md"] {
            let page = std::fs::read_to_string(docs.join(file)).unwrap_or_else(|err| panic!("{}: {}", file, err));
            let page = normalize(&page);
            for builtin in BUILTINS {
                assert!(page.contains(&format!("`{}`", builtin.name)), "{} does not list {}", file, builtin.name);
                assert!(
                    page.contains(&normalize(builtin.bql)),
                    "{} does not show the BQL of {} as it is",
                    file,
                    builtin.name
                );
            }
        }
    }

    #[test]
    fn json_values_bind_with_the_declared_types() {
        use BuiltinParamValue::*;
        let ok = |ty, value| json_value(ty, Some(value)).unwrap();
        assert_eq!(ok(DataType::Bool, Bool(false)), Value::Bool(false));
        assert_eq!(ok(DataType::Int, Int(-4)), Value::Int(-4));
        assert_eq!(ok(DataType::Decimal, Int(12)).to_string(), "12");
        assert_eq!(ok(DataType::Decimal, Number(12.5)).to_string(), "12.5");
        assert_eq!(ok(DataType::Decimal, Number(0.1)).to_string(), "0.1");
        assert_eq!(ok(DataType::Decimal, Text(" 12.50 ".into())).to_string(), "12.50");
        assert_eq!(ok(DataType::Str, Text("x".into())), Value::Str("x".into()));
        assert_eq!(ok(DataType::Date, Text("2024-02-29".into())).to_string(), "2024-02-29");
        assert_eq!(ok(DataType::Set, List(vec!["b".into(), "a".into(), "b".into()])).to_string(), "a, b");
        for ty in JSON_TYPES {
            assert_eq!(json_value(ty, None).unwrap(), Value::Null);
        }
        assert_eq!(json_value(DataType::Int, Some(Number(1.5))).unwrap_err(), "an integer or null");
        assert_eq!(json_value(DataType::Int, Some(Text("1".into()))).unwrap_err(), "an integer or null");
        assert_eq!(
            json_value(DataType::Date, Some(Text("2024-02-30".into()))).unwrap_err(),
            "a date string YYYY-MM-DD or null"
        );
        assert_eq!(
            json_value(DataType::Date, Some(Text("2024-02-01T00:00:00Z".into()))).unwrap_err(),
            "a date string YYYY-MM-DD or null"
        );
        assert_eq!(json_value(DataType::Set, Some(Text("a".into()))).unwrap_err(), "a list of strings or null");
        assert!(json_value(DataType::Decimal, Some(Text("twelve".into()))).is_err());
        assert!(json_value(DataType::Str, Some(Int(1))).is_err());
    }

    #[test]
    fn json_params_need_every_declared_parameter_and_no_other() {
        let builtin = super::get("postings.between").unwrap();
        let values = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, value)| (name.to_string(), Some(BuiltinParamValue::Text(value.to_string()))))
                .collect::<HashMap<_, _>>()
        };
        let params = json_params(builtin, values(&[("from", "2024-01-01"), ("to", "2024-01-31")])).unwrap();
        assert_eq!(params.types(), builtin.param_types());
        let error = json_params(builtin, values(&[("from", "2024-01-01")])).unwrap_err();
        assert_eq!(error.to_string(), "parameter :to of postings.between is missing");
        let error = json_params(builtin, values(&[("from", "2024-01-01"), ("to", "2024-01-31"), ("zz", "x"), ("too", "x")])).unwrap_err();
        assert_eq!(error.to_string(), "postings.between has no parameter :too");
        let error = json_params(builtin, values(&[("from", "2024-01-01"), ("to", "soon")])).unwrap_err();
        assert_eq!(error.to_string(), "parameter :to must be a date string YYYY-MM-DD or null");
    }
}
