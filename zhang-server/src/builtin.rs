//! Built-in queries: the named, documented BQL behind the figures the app shows (#479).
//!
//! The app computes its figures with these queries, so the business logic lives in the query
//! engine and every figure can be opened, and adapted, on the Query page (`/explore`):
//! `GET /api/query/builtins` lists the queries with their parameters and columns,
//! `POST /api/query/builtins/{name}` runs one with its parameters bound (and
//! `POST /api/query/builtins` several under one read lock), `POST /api/query/builtins/{name}/text`
//! writes one out with its parameters filled in, and the "Built-in queries" page of the docs
//! lists them with their BQL. A read endpoint that still maps a result into its own response
//! runs them with [`run`] or [`execute`].
//!
//! # Adding a query
//!
//! 1. Add a [`BuiltinQuery`] to [`BUILTINS`], in the section of the endpoints that run it: a
//!    unique dotted lower-case name (`report.summary`), a one-sentence description, the BQL,
//!    and every parameter it uses with its type. Parameters are named (`:from`), and their
//!    types are those a JSON value can give (see [`json_params`]).
//! 2. The frontend runs it with `runBuiltin(name, params)` and gets typed rows: regenerate its
//!    types, `frontend/src/api/builtins.ts`, with
//!    `ZHANG_WRITE_BUILTINS_TS=1 cargo test -p zhang-server the_frontend_types_of_the_builtins_are_up_to_date`.
//!    On the server, run it with [`run`], or with [`execute`] inside [`crate::routes::query::with_ledger`]
//!    to run several under one read lock, and map the [`QueryResult`] into the endpoint's response;
//!    [`calculated_amount`] and [`LedgerDateRange`] are the shared pieces of that mapping. User
//!    input is only ever bound as a parameter, never formatted into the BQL.
//! 3. List the query, with its BQL as it is here, on the "Built-in queries" page of the docs
//!    (`docs/src/content/docs/reference/builtin-queries.md` and its `zh-cn` twin).
//!
//! The tests check that every query compiles, declares exactly the parameters its BQL uses,
//! can be written out as BQL, is documented, and has its frontend types up to date.
//!
//! # Queries of providers
//!
//! The providers of a ledger's tables declare named queries too ([`Catalog::named_queries`]): `zhang:budget` declares
//! those of the budget pages, `budgets.*`, and an analyzer may declare its own, named after its namespace. The built-in
//! queries of a ledger are zhang's own, [`BUILTINS`], and those of its catalog's providers, listed among zhang's before
//! [`PROVIDED_BEFORE`]; zhang's own keep their names. They are compiled once per catalog ([`compiled_for`]), and the
//! endpoints serve those of the ledger served.

mod amount;
mod date_range;

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use chrono::{Datelike, NaiveDateTime};
use zhang_core::ledger::Ledger;
use zhang_query::params::from_json;
use zhang_query::{Catalog, CatalogDigest, DataType, ExecuteOptions, NamedQuery, ParamTypes, Params, Query, QueryError, QueryResult, Value};

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

    /// The query as the registry of a catalog serves it, with the queries of the catalog's providers.
    pub fn named(&self) -> NamedQuery {
        NamedQuery {
            name: self.name.to_owned(),
            description: self.description.to_owned(),
            bql: self.bql.to_owned(),
            params: self.params.iter().map(|(name, ty)| ((*name).to_owned(), *ty)).collect(),
        }
    }
}

/// The columns of `#balances` both journals describe a balance assertion with, in one projection, so the journal and an
/// account's journal show an assertion alike: the account, the asserted `amount` and its `tolerance`, the balance it was
/// checked against (`actual`), the asserted amount minus that balance (`difference`) and whether it held (`passed`).
macro_rules! assertion_columns {
    () => {
        "account, amount, tolerance, actual, amount - actual AS difference, passed"
    };
}

// A built-in query and its variant for one account subtree, budget or commodity share their BQL through these macros.

/// `accounts.list`, and with a filter `accounts.subtree`.
macro_rules! account_rows {
    ($($filter:literal)?) => { concat!("SELECT account, open, close, meta('alias') AS alias, account_status(account, :date, :time) AS status
FROM #accounts
", $($filter, "\n",)? "ORDER BY account") };
}

/// `accounts.balances`, and with a filter `accounts.subtree_balances`.
macro_rules! account_balances {
    ($($filter:literal)?) => { concat!("SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
", $($filter, "\n",)? "GROUP BY account, currency
ORDER BY account, currency") };
}

/// `accounts.journal`, and with a `LIMIT` `accounts.journal_page`.
macro_rules! account_journal {
    ($($limit:literal)?) => { concat!("SELECT first(date) AS date, first(time) AS time, first(timestamp) AS timestamp, first(flag) AS flag,
       first(id) AS id, first(account) AS account, first(payee) AS payee, first(narration) AS narration,
       seq, posting_index, sum(number) AS units, first(currency) AS currency, last(units(balance)) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index", $("\n", $limit)?) };
}

/// The postings of the accounts that hold commodities, those of Assets and Liabilities.
macro_rules! held {
    () => {
        "under(account, 'Assets') OR under(account, 'Liabilities')"
    };
}

/// `commodities.totals` without its order, and with a filter `commodities.total`.
macro_rules! commodity_totals {
    ($($filter:expr),+) => { concat!("SELECT currency, sum(number) AS total WHERE ", $($filter,)+ " GROUP BY currency HAVING sum(number) != 0") };
}

/// `commodities.latest_prices` without its order, and for one commodity `commodities.latest_price`.
macro_rules! latest_prices {
    ($rate_of:literal, $pairs:literal) => {
        concat!(
            "SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency, \
              last(date) AS date, last(time) AS time, getprice(",
            $rate_of,
            ", :currency, today()) AS rate FROM #prices WHERE ",
            $pairs,
            " AND currency != currency(amount) AND date <= today() GROUP BY 1"
        )
    };
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
    BuiltinQuery {
        name: "ledger.now",
        description: "The ledger's current date and time of day by its clock in its timezone, what today() and now() give, as one row; the forms take the default date and time of what they write from it.",
        bql: "SELECT today() AS date, now() AS time FROM #accounts LIMIT 1",
        params: &[],
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
        description: "Every account with an open or close directive, with its open and close dates, its alias and its status at a \
                      date and time, by name. The account list asks for now.",
        bql: account_rows!(),
        params: &[("date", DataType::Date), ("time", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.balances",
        description: "The balance of every account of its own postings per currency: the units, their value in the operating currency \
                      at today's prices, and the date of the first posting.",
        bql: account_balances!(),
        params: &[("operating_currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.subtree",
        description: "An account and those of its sub-accounts that have an open or close directive, with their open and close dates, \
                      their aliases and their statuses at a date and time, by name. The account page asks for now.",
        bql: account_rows!("WHERE under(account, :account)"),
        params: &[("account", DataType::Str), ("date", DataType::Date), ("time", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.opened",
        description: "The accounts opened by a date and time, closed ones included, by name: those a document or a balance assertion written then may name, as a record of an account may follow its close. The rule the ledger checks such directives with: an open at or before the instant.",
        bql: "SELECT account \
              FROM #accounts \
              WHERE open.date < :date OR (open.date = :date AND open.time <= :time) \
              ORDER BY account",
        params: &[("date", DataType::Date), ("time", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.subtree_balances",
        description: "The balance of an account and of each of its sub-accounts of their own postings per currency: the units, their \
                      value in the operating currency at today's prices, and the date of the first posting.",
        bql: account_balances!("WHERE under(account, :account)"),
        params: &[("account", DataType::Str), ("operating_currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.journal",
        description: "The postings of an account and its sub-accounts in ledger order, a row per posting, the lots it is booked \
                      against added up, each with the running balance of the account and its sub-accounts right after it.",
        bql: account_journal!(),
        params: &[("account", DataType::Str)],
    },
    BuiltinQuery {
        name: "accounts.journal_page",
        description: "Some rows of accounts.journal: from an offset, at most a limit of them.",
        bql: account_journal!("LIMIT :limit OFFSET :offset"),
        params: &[("account", DataType::Str), ("limit", DataType::Int), ("offset", DataType::Int)],
    },
    BuiltinQuery {
        name: "accounts.balance_assertions",
        description: "The balance assertions on an account, newest first, each as journals.balance_checks describes it, with the \
                      account a balance with pad pads from.",
        bql: concat!(
            "SELECT date, time, timestamp, id, ",
            assertion_columns!(),
            ", pad, seq
FROM #balances
WHERE account = :account
ORDER BY seq DESC"
        ),
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
    // ---- journals: /api/journals, the new-transaction form, and the documents and errors pages ----
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
        description: "Some balance assertions with the asserted amount, its tolerance, the balance of the account and its \
                      sub-accounts it was checked against, their difference and whether the assertion holds.",
        bql: concat!("SELECT id, ", assertion_columns!(), " FROM #balances WHERE id IN :ids"),
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
        description: "The accounts open at a date and time, sorted by name: those a transaction written then may post to.",
        bql: "SELECT account \
              FROM #accounts \
              WHERE account_status(account, :date, :time) = 'open' \
              ORDER BY account",
        params: &[("date", DataType::Date), ("time", DataType::Str)],
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
    // ---- the queries of the catalog's providers go here (`PROVIDED_BEFORE`): `zhang:budget`'s budgets.* ----
    // ---- commodities: /api/commodities/* ----
    BuiltinQuery {
        name: "commodities.totals",
        description: "How many units of each commodity the Assets and Liabilities accounts hold, for the commodities they hold.",
        bql: concat!(commodity_totals!(held!()), " ORDER BY currency"),
        params: &[],
    },
    BuiltinQuery {
        name: "commodities.total",
        description: "How many units of a commodity the Assets and Liabilities accounts hold; no row if they hold none.",
        bql: commodity_totals!("currency = :commodity AND (", held!(), ")"),
        params: &[("commodity", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.latest_prices",
        description: "The latest price of each commodity in a currency as of today, with its date and time; the rate is the one the valuations use (the latest quote of the pair on or before today, in either direction, inverted when it is quoted the other way round).",
        bql: concat!(
            latest_prices!(
                "last(CASE WHEN currency = :currency THEN currency(amount) ELSE currency END)",
                "(currency = :currency OR currency(amount) = :currency)"
            ),
            " ORDER BY 1"
        ),
        params: &[("currency", DataType::Str)],
    },
    BuiltinQuery {
        name: "commodities.latest_price",
        description: "The latest price of a commodity in a currency as of today, with its date and time; the rate is the one the valuations use (the latest quote of the pair on or before today, in either direction, inverted when it is quoted the other way round).",
        bql: latest_prices!(
            ":commodity",
            "((currency = :commodity AND currency(amount) = :currency) OR (currency = :currency AND currency(amount) = :commodity))"
        ),
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

/// Where the named queries of a catalog's providers are listed among zhang's own: right before this one. The queries of
/// the budget pages, which `zhang:budget` declares, were zhang's own until RFC #698 and listed there, after those of
/// the journals and before those of the commodities: `GET /api/query/builtins` and the frontend's types
/// (`frontend/src/api/builtins.ts`) keep that order.
pub const PROVIDED_BEFORE: &str = "commodities.totals";

/// zhang's own built-in query named `name`; a provider's is in the registry of a catalog ([`Compiled::named`]).
pub fn get(name: &str) -> Option<&'static BuiltinQuery> {
    BUILTINS.iter().find(|query| query.name == name)
}

/// A built-in query of a catalog: zhang's own or a provider's, and the query compiled against the catalog.
pub struct Registered {
    pub query: NamedQuery,
    /// the provider declaring it, such as `zhang:budget`; `None` for zhang's own
    pub provided_by: Option<String>,
    /// the query compiled against the catalog, or why it does not compile
    compiled: Result<Query, QueryError>,
}

impl Registered {
    /// The query compiled against the catalog; a query error when it does not compile against it.
    pub fn compiled(&self) -> ServerResult<&Query> {
        self.compiled.as_ref().map_err(|error| error.clone().into())
    }

    /// Execute the query on `ledger`, whose catalog it was compiled against, as [`execute`] does.
    pub fn execute(&self, ledger: &Ledger, params: &Params, count_total: bool) -> ServerResult<QueryResult> {
        let options = ExecuteOptions {
            count_total,
            ..execute_options(max_result_values())
        };
        Ok(self.compiled()?.execute_with_options(ledger, params, &options)?)
    }
}

/// The built-in queries of one catalog, compiled against it: zhang's own and those of the catalog's providers. A query
/// to run on a ledger compiles against the ledger's catalog ([`Catalog::of_ledger`]), which holds the tables of its
/// analyzers too.
pub struct Compiled {
    digest: CatalogDigest,
    /// every built-in query, in the order they are listed: zhang's own, with the providers' before [`PROVIDED_BEFORE`]
    queries: Vec<Registered>,
    /// the position of each in `queries`, by name
    names: HashMap<String, usize>,
}

impl Compiled {
    /// Every built-in query of `catalog`, compiled against it. A provider's query named like one of zhang's own is left
    /// out: zhang's own keep their names (the catalog gives an analyzer no namespace of theirs).
    fn of(catalog: &Catalog) -> Compiled {
        let mut own = BUILTINS.iter().map(|builtin| (builtin.named(), None)).collect::<Vec<_>>();
        let provided = catalog
            .named_queries()
            .filter(|(_, query)| get(&query.name).is_none())
            .map(|(provider, query)| (query.clone(), Some(provider.to_owned())));
        let at = BUILTINS.iter().position(|builtin| builtin.name == PROVIDED_BEFORE).unwrap_or(own.len());
        let after = own.split_off(at);
        let ordered = own.into_iter().chain(provided).chain(after).collect::<Vec<_>>();
        let queries = ordered
            .into_iter()
            .map(|(query, provided_by)| {
                let compiled = Query::compile_in(catalog, &query.bql, &query.param_types());
                Registered { query, provided_by, compiled }
            })
            .collect::<Vec<_>>();
        let names = queries.iter().enumerate().map(|(at, it)| (it.query.name.clone(), at)).collect();
        Compiled {
            digest: catalog.digest(),
            queries,
            names,
        }
    }

    /// Every built-in query, in the order `GET /api/query/builtins` lists them.
    pub fn queries(&self) -> &[Registered] {
        &self.queries
    }

    /// The built-in query named `name`, zhang's own or a provider's.
    pub fn named(&self, name: &str) -> Option<&Registered> {
        self.names.get(name).map(|at| &self.queries[*at])
    }

    /// The compiled built-in query named `name`; a query error when it does not compile against the catalog.
    pub fn get(&self, name: &str) -> ServerResult<&Query> {
        self.named(name).ok_or_else(|| ServerError::UnknownBuiltinQuery(name.to_owned()))?.compiled()
    }
}

/// How many catalogs other than the standard one the built-in queries are kept compiled for. A ledger keeps one catalog
/// across reloads (its analyzers' manifests change only with their modules), so this is the catalogs of the ledgers a
/// server served lately.
const COMPILED_CATALOGS: usize = 8;

/// The built-in queries compiled against the standard catalog ([`Catalog::standard`]), once, on first use.
static STANDARD: LazyLock<Arc<Compiled>> = LazyLock::new(|| {
    let compiled = Compiled::of(Catalog::standard());
    for registered in &compiled.queries {
        // the tests compile every built-in query, so this cannot fail in a release
        if let Err(err) = &registered.compiled {
            panic!("the built-in query {} does not compile: {}", registered.query.name, err);
        }
    }
    Arc::new(compiled)
});

/// The built-in queries compiled against other catalogs, the latest last.
static COMPILED: Mutex<Vec<Arc<Compiled>>> = Mutex::new(Vec::new());

/// The built-in queries of the standard catalog, that of a ledger without tables of its own: zhang's own and the
/// native providers'.
pub fn standard() -> &'static Compiled {
    &STANDARD
}

/// The built-in query named `name` compiled against the standard catalog: its parameters, columns and text.
pub fn compiled(name: &str) -> ServerResult<&'static Query> {
    STANDARD.get(name)
}

/// The built-in queries compiled against the catalog of `ledger` ([`Catalog::of_ledger`]): compiled once per catalog,
/// so a reload of the ledger, which keeps its catalog, compiles nothing again, and a query never runs on a ledger whose
/// tables it was not compiled for.
pub fn compiled_for(ledger: &Ledger) -> Arc<Compiled> {
    let catalog = Catalog::of_ledger(ledger);
    if catalog.digest() == STANDARD.digest {
        return STANDARD.clone();
    }
    let lock = || COMPILED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(compiled) = lock().iter().find(|compiled| compiled.digest == catalog.digest()) {
        return compiled.clone();
    }
    // compiled outside the lock: another catalog's queries never wait for these
    let compiled = Arc::new(Compiled::of(&catalog));
    let mut cached = lock();
    if !cached.iter().any(|it| it.digest == compiled.digest) {
        if cached.len() == COMPILED_CATALOGS {
            cached.remove(0);
        }
        cached.push(compiled.clone());
    }
    compiled
}

/// Execute the built-in query `name` on `ledger`, compiled against its catalog, with the limits of every query the
/// server runs (the time limit and the result size limit of `POST /api/query`). With `count_total` the result's
/// `total` is the number of rows before `LIMIT` and `OFFSET`.
///
/// This blocks: call it inside [`with_ledger`], which also runs several queries under one read
/// lock, or use [`run`].
pub fn execute(ledger: &Ledger, name: &str, params: &Params, count_total: bool) -> ServerResult<QueryResult> {
    let compiled = compiled_for(ledger);
    compiled
        .named(name)
        .ok_or_else(|| ServerError::UnknownBuiltinQuery(name.to_owned()))?
        .execute(ledger, params, count_total)
}

/// Now by the ledger's clock, as a wall-clock date and time of its timezone.
pub(crate) fn ledger_now(ledger: &Ledger) -> NaiveDateTime {
    ledger.now().with_timezone(&ledger.options.timezone).naive_local()
}

/// `params` with `:date` and `:time` bound to `at`, a wall-clock date and time of the ledger, for the built-in queries
/// that ask what accounts are open then.
pub(crate) fn bind_instant(params: Params, at: NaiveDateTime) -> Params {
    params
        .bind("date", Value::Date(at.date()))
        .bind("time", Value::Str(at.time().format("%H:%M:%S").to_string()))
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
/// no other. Each value is read by the engine's [`from_json`]: `null` is NULL for any type;
/// otherwise a `bool` takes a boolean, an `int` an integer, a `decimal` a number or a string
/// such as `"12.50"`, a `str` a string, a `date` a string `YYYY-MM-DD` read as BQL's
/// `date(text)` reads it, and a `set` a list of strings. A `month` (of the budget queries) is the
/// first day of a month; another day is refused, naming the first day of its month.
pub fn json_params(builtin: &NamedQuery, mut values: HashMap<String, Option<BuiltinParamValue>>) -> ServerResult<Params> {
    let mut params = Params::new();
    for (name, ty) in &builtin.params {
        let value = values
            .remove(name)
            .ok_or_else(|| ServerError::InvalidInput(format!("parameter :{} of {} is missing", name, builtin.name)))?;
        let value = value.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
        let value = from_json(*ty, &value).map_err(|error| ServerError::InvalidInput(error.for_param(name)))?;
        // a month is given as its first day: another day of the month would answer for that month carried over
        if let (MONTH_PARAM, Value::Date(date)) = (name.as_str(), &value) {
            if date.day() != 1 {
                let first = date.with_day(1).expect("every month has a first day");
                return Err(ServerError::InvalidInput(format!(
                    "parameter :{} of {} must be the first day of a month, such as {}, not {}",
                    name, builtin.name, first, date
                )));
            }
        }
        params = params.bind(name.as_str(), value);
    }
    match values.keys().collect::<BTreeSet<_>>().first() {
        Some(unknown) => Err(ServerError::InvalidInput(format!("{} has no parameter :{}", builtin.name, unknown))),
        None => Ok(params),
    }
}

/// The parameter of the budget queries that names a month: its first day, such as `2024-06-01`.
const MONTH_PARAM: &str = "month";

/// The BQL of `builtin` with `params` written in as literals: a query that runs on the Query
/// page (`/explore`), where parameters cannot be bound, to the same result. Display only; the
/// app itself always binds them.
pub fn text(builtin: &Registered, params: &Params) -> ServerResult<String> {
    Ok(builtin.compiled()?.inline_params(params)?)
}

#[cfg(test)]
mod test {
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;

    use zhang_ast::Flag;
    use zhang_core::constants::BALANCE_CHECK_PAYEE;
    use zhang_query::params::JSON_TYPES;
    use zhang_query::{DataType, NamedQuery, ParamRef, Query};

    use super::{compiled, json_params, standard, text, BUILTINS, PROVIDED_BEFORE};
    use crate::request::BuiltinParamValue;

    /// Every built-in query of a ledger without tables of its own, zhang's and the native providers', in order.
    fn every_builtin() -> impl Iterator<Item = &'static NamedQuery> {
        standard().queries().iter().map(|registered| &registered.query)
    }

    /// A value of the parameter `name` of type `ty`: a month is its first day.
    fn param_sample(name: &str, ty: DataType) -> BuiltinParamValue {
        match name {
            super::MONTH_PARAM => BuiltinParamValue::Text("2024-02-01".to_owned()),
            _ => sample(ty),
        }
    }

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
        for builtin in every_builtin() {
            let query = Query::compile_with_params(&builtin.bql, &builtin.param_types()).unwrap_or_else(|err| panic!("{}: {}", builtin.name, err));
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
            assert_eq!(compiled(&builtin.name).unwrap().source(), builtin.bql);
        }
    }

    /// The queries of the budget pages are `zhang:budget`'s, listed where they were when they were zhang's own: after the
    /// journals' and before the commodities'. Every other built-in query is zhang's own.
    #[test]
    fn the_budget_pages_queries_are_zhang_budgets_listed_where_they_always_were() {
        let listed = standard()
            .queries()
            .iter()
            .map(|it| (it.query.name.as_str(), it.provided_by.as_deref()))
            .collect::<Vec<_>>();
        let budgets = listed.iter().position(|(name, _)| *name == "budgets.month").unwrap();
        let provided = listed.iter().filter(|(_, provider)| provider.is_some()).copied().collect::<Vec<_>>();
        assert_eq!(
            provided,
            ["budgets.month", "budgets.budget", "budgets.budget_month", "budgets.events", "budgets.postings"].map(|name| (name, Some("zhang:budget")))
        );
        assert_eq!(listed[budgets - 1].0, "journals.errors");
        assert_eq!(listed[budgets + provided.len()].0, PROVIDED_BEFORE);
        assert_eq!(listed.len(), BUILTINS.len() + provided.len());
        assert!(
            BUILTINS.iter().all(|it| !it.name.starts_with("budgets.")),
            "the budget queries are zhang:budget's"
        );
    }

    /// An analyzer names its queries after its namespace, which is never one of zhang's own built-in queries
    /// ([`zhang_query::BUILTIN_QUERY_NAMESPACES`]), so that zhang's keep their names.
    #[test]
    fn the_namespaces_of_the_builtins_are_reserved() {
        let namespaces = every_builtin().map(|it| it.name.split('.').next().unwrap().to_owned()).collect::<BTreeSet<_>>();
        let reserved = zhang_query::BUILTIN_QUERY_NAMESPACES.iter().map(|it| (*it).to_owned()).collect::<BTreeSet<_>>();
        assert_eq!(namespaces, reserved);
    }

    /// A query cannot read a Rust constant, so the built-in queries write two of the ledger's as literals: the payee a
    /// balance assertion is listed under in the journals, and the flag of padding transactions. Each literal is the
    /// constant, and nothing else names that payee or compares the flag with another
    #[test]
    fn the_literals_builtins_copy_are_the_ledger_constants() {
        let payee = format!("'{}'", BALANCE_CHECK_PAYEE);
        let not_padding = format!("flag != '{}'", Flag::BalancePad);
        let named = |text: &str| {
            every_builtin()
                .filter(|it| it.bql.contains(text))
                .map(|it| it.name.as_str())
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(named(&payee), BTreeSet::from(["journals.page"]));
        assert_eq!(named("Balance Check"), named(&payee));
        assert_eq!(named(&not_padding), BTreeSet::from(["journals.payees", "report.transaction_count"]));
        assert_eq!(named("flag != '"), named(&not_padding));
        assert_eq!(named("flag = '"), BTreeSet::new());
    }

    #[test]
    fn builtin_names_are_unique_dotted_and_lower_case_and_described() {
        let mut names = BTreeSet::new();
        for NamedQuery { name, description, .. } in every_builtin() {
            assert!(names.insert(name), "{} is registered twice", name);
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
        for registered in standard().queries() {
            let builtin = &registered.query;
            for (name, ty) in &builtin.params {
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
                    .map(|(name, ty)| (name.to_string(), (!null || required(name)).then(|| param_sample(name, *ty))))
                    .collect::<HashMap<_, _>>();
                let params = json_params(builtin, values).unwrap();
                let written = text(registered, &params).unwrap_or_else(|err| panic!("{}: {}", builtin.name, err));
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
            for builtin in every_builtin() {
                assert!(page.contains(&format!("`{}`", builtin.name)), "{} does not list {}", file, builtin.name);
                assert!(
                    page.contains(&normalize(&builtin.bql)),
                    "{} does not show the BQL of {} as it is",
                    file,
                    builtin.name
                );
            }
        }
    }

    /// `frontend/src/api/builtins.ts`, as prettier formats it: the `params` and `row` types of every built-in query,
    /// for the frontend's `runBuiltin`. Every cell may be `null` (the engine has no NOT NULL), and so may a parameter.
    fn frontend_types() -> String {
        /// a cell of the type, as `POST /api/query` encodes it (`QueryCell`)
        fn ts(ty: DataType) -> &'static str {
            match ty {
                DataType::Null => "null",
                DataType::Bool => "boolean",
                DataType::Int => "number",
                // decimals, dates and intervals are strings on the wire
                DataType::Decimal | DataType::Str | DataType::Date | DataType::Interval => "string",
                DataType::Set => "string[]",
                DataType::Amount => "QueryAmount",
                DataType::Position => "QueryPosition",
                DataType::Inventory => "QueryInventory",
                DataType::Metas => "QueryMeta[]",
            }
        }
        /// a parameter's JSON value of the type (`json_params`): a decimal may also be a number
        fn ts_param(ty: DataType) -> &'static str {
            match ty {
                DataType::Decimal => "number | string",
                other => ts(other),
            }
        }
        /// a property name, quoted unless it is an identifier
        fn key(name: &str) -> String {
            let identifier = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_' || c == '$')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
            if identifier {
                name.to_owned()
            } else {
                format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'"))
            }
        }
        /// an object type, a property per line, or `Record<string, never>` for none
        fn object(fields: Vec<(String, &str)>) -> String {
            if fields.is_empty() {
                return "Record<string, never>".to_owned();
            }
            let fields = fields
                .into_iter()
                .map(|(name, ty)| format!("      {}: {} | null;\n", name, ty))
                .collect::<String>();
            format!("{{\n{}    }}", fields)
        }
        let mut out = String::new();
        let mut imports = BTreeSet::new();
        for builtin in every_builtin() {
            let columns = compiled(&builtin.name).unwrap().columns();
            let names = columns.iter().map(|column| column.name.as_str()).collect::<BTreeSet<_>>();
            assert_eq!(
                names.len(),
                columns.len(),
                "{} selects a column twice, so its rows cannot be read by name",
                builtin.name
            );
            for ty in builtin.params.iter().map(|(_, ty)| *ty).chain(columns.iter().map(|column| column.ty)) {
                if let Some(entity) = ts(ty).strip_suffix("[]").or(Some(ts(ty))).filter(|it| it.starts_with("Query")) {
                    imports.insert(entity);
                }
            }
            let params = object(builtin.params.iter().map(|(name, ty)| (key(name), ts_param(*ty))).collect());
            let row = object(columns.iter().map(|column| (key(&column.name), ts(column.ty))).collect());
            out.push_str(&format!("  '{}': {{\n    params: {};\n    row: {};\n  }};\n", builtin.name, params, row));
        }
        format!(
            "// Generated from the built-in queries (zhang-server/src/builtin.rs) by their tests; do not edit.\n\
             // Regenerate with `ZHANG_WRITE_BUILTINS_TS=1 cargo test -p zhang-server the_frontend_types_of_the_builtins_are_up_to_date`.\n\
             import type {{ {} }} from './types';\n\n\
             /**\n \
             * Every built-in query (`GET /api/query/builtins`): the `params` that `POST /api/query/builtins/{{name}}` takes, and\n \
             * the `row` it returns, by column name (`runBuiltin` in requests.ts). Any cell may be `null`.\n \
             */\n\
             export interface Builtins {{\n{}}}\n",
            imports.into_iter().collect::<Vec<_>>().join(", "),
            out
        )
    }

    /// The TypeScript types of every built-in query's parameters and rows, `frontend/src/api/builtins.ts`: written
    /// with `ZHANG_WRITE_BUILTINS_TS=1`, and otherwise checked to be up to date, so a change of a query's columns
    /// fails here first, and then in the frontend's type check wherever a page reads a column that went.
    #[test]
    fn the_frontend_types_of_the_builtins_are_up_to_date() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend/src/api/builtins.ts");
        let generated = frontend_types();
        if std::env::var_os("ZHANG_WRITE_BUILTINS_TS").is_some() {
            std::fs::write(&path, &generated).unwrap();
        }
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            current == generated,
            "frontend/src/api/builtins.ts is out of date: run \
             `ZHANG_WRITE_BUILTINS_TS=1 cargo test -p zhang-server the_frontend_types_of_the_builtins_are_up_to_date`"
        );
    }

    #[test]
    fn json_params_need_every_declared_parameter_and_no_other() {
        let builtin = &super::get("postings.between").unwrap().named();
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

    /// A budget query's month is its first day: another day of the month would answer for the month carried over
    /// (budgets.month) or for no month at all (budgets.events), so it is refused.
    #[test]
    fn a_month_is_the_first_day_of_a_month() {
        let month = |name: &str, value: &str| {
            let builtin = &standard().named(name).unwrap().query;
            let values = builtin
                .params
                .iter()
                .map(|(param, ty)| {
                    let value = if *param == super::MONTH_PARAM {
                        BuiltinParamValue::Text(value.to_owned())
                    } else {
                        sample(*ty)
                    };
                    (param.to_string(), Some(value))
                })
                .collect::<HashMap<_, _>>();
            json_params(builtin, values)
        };
        for name in ["budgets.month", "budgets.budget_month", "budgets.events", "budgets.postings"] {
            assert!(month(name, "2024-03-01").is_ok(), "{}", name);
            assert_eq!(
                month(name, "2024-03-15").unwrap_err().to_string(),
                format!(
                    "parameter :month of {} must be the first day of a month, such as 2024-03-01, not 2024-03-15",
                    name
                )
            );
        }
    }
}
