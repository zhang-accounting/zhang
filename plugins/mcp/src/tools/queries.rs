//! The BQL the tools run.
//!
//! Where zhang-server has a built-in query for the figure (`zhang-server/src/builtin.rs`, the "Built-in queries"
//! page of the docs), its text is copied here verbatim, so the MCP answers stay the app's answers: a parity test
//! in zhang-server (`tests/mcp_plugin.rs`) pins every [`NamedQuery`] marked `builtin` to `BUILTINS`. The
//! remaining queries are written in the same style: parameters are named, a NULL parameter leaves its filter out,
//! and user input only ever reaches the engine as a bound parameter, never formatted into the text.

/// the type of a parameter, as the `zhang_query_params` host function names it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Int,
    Str,
    Date,
    Set,
}

/// a query of the catalogue, with every parameter it uses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedQuery {
    /// the name of the built-in query it copies, or a `mcp.` name of its own
    pub name: &'static str,
    /// whether `name` is a built-in query of zhang-server whose text this is, verbatim
    pub builtin: bool,
    pub bql: &'static str,
    pub params: &'static [(&'static str, ParamType)],
}

// ---- copied verbatim from zhang-server's BUILTINS ----

pub const ACCOUNTS_LIST: NamedQuery = NamedQuery {
    name: "accounts.list",
    builtin: true,
    bql: "SELECT account, open, close, meta('alias') AS alias, account_status(account, :date, :time) AS status
FROM #accounts
ORDER BY account",
    params: &[("date", ParamType::Date), ("time", ParamType::Str)],
};

pub const ACCOUNTS_SUBTREE: NamedQuery = NamedQuery {
    name: "accounts.subtree",
    builtin: true,
    bql: "SELECT account, open, close, meta('alias') AS alias, account_status(account, :date, :time) AS status
FROM #accounts
WHERE under(account, :account)
ORDER BY account",
    params: &[("account", ParamType::Str), ("date", ParamType::Date), ("time", ParamType::Str)],
};

pub const ACCOUNTS_BALANCES: NamedQuery = NamedQuery {
    name: "accounts.balances",
    builtin: true,
    bql: "SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
GROUP BY account, currency
ORDER BY account, currency",
    params: &[("operating_currency", ParamType::Str)],
};

pub const ACCOUNTS_SUBTREE_BALANCES: NamedQuery = NamedQuery {
    name: "accounts.subtree_balances",
    builtin: true,
    bql: "SELECT account, currency, sum(number) AS units,
       convert(sum(position), :operating_currency, today()) AS value,
       min(date) AS first_date
WHERE under(account, :account)
GROUP BY account, currency
ORDER BY account, currency",
    params: &[("account", ParamType::Str), ("operating_currency", ParamType::Str)],
};

pub const ACCOUNTS_JOURNAL_PAGE: NamedQuery = NamedQuery {
    name: "accounts.journal_page",
    builtin: true,
    bql: "SELECT first(date) AS date, first(time) AS time, first(timestamp) AS timestamp, first(flag) AS flag,
       first(id) AS id, first(account) AS account, first(payee) AS payee, first(narration) AS narration,
       seq, posting_index, sum(number) AS units, first(currency) AS currency, last(units(balance)) AS balance
WHERE under(account, :account)
GROUP BY seq, posting_index
LIMIT :limit OFFSET :offset",
    params: &[("account", ParamType::Str), ("limit", ParamType::Int), ("offset", ParamType::Int)],
};

pub const JOURNALS_PAYEES: NamedQuery = NamedQuery {
    name: "journals.payees",
    builtin: true,
    bql: "SELECT DISTINCT payee \
              FROM #transactions \
              WHERE payee IS NOT NULL AND payee != '' AND flag != 'P' \
              ORDER BY payee",
    params: &[],
};

pub const JOURNALS_ERRORS: NamedQuery = NamedQuery {
    name: "journals.errors",
    builtin: true,
    bql: "SELECT id, kind, file, line, column, span_start, span_end, source, metas \
              FROM #errors \
              LIMIT :size OFFSET :offset",
    params: &[("size", ParamType::Int), ("offset", ParamType::Int)],
};

pub const COMMODITIES_LATEST_PRICES: NamedQuery = NamedQuery {
    name: "commodities.latest_prices",
    builtin: true,
    bql: "SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency, \
              last(date) AS date, last(time) AS time, getprice(last(CASE WHEN currency = :currency THEN currency(amount) ELSE currency END), :currency, today()) AS rate FROM #prices WHERE (currency = :currency OR currency(amount) = :currency) AND currency != currency(amount) AND date <= today() GROUP BY 1 ORDER BY 1",
    params: &[("currency", ParamType::Str)],
};

pub const COMMODITIES_LATEST_PRICE: NamedQuery = NamedQuery {
    name: "commodities.latest_price",
    builtin: true,
    bql: "SELECT CASE WHEN currency = :currency THEN currency(amount) ELSE currency END AS currency, \
              last(date) AS date, last(time) AS time, getprice(:commodity, :currency, today()) AS rate FROM #prices WHERE ((currency = :commodity AND currency(amount) = :currency) OR (currency = :currency AND currency(amount) = :commodity)) AND currency != currency(amount) AND date <= today() GROUP BY 1",
    params: &[("commodity", ParamType::Str), ("currency", ParamType::Str)],
};

pub const REPORT_NET_WORTH: NamedQuery = NamedQuery {
    name: "report.net_worth",
    builtin: true,
    bql: "SELECT sum(position) AS balance, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE (under(account, 'Assets') OR under(account, 'Liabilities')) AND date <= :to",
    params: &[("to", ParamType::Date), ("currency", ParamType::Str)],
};

pub const REPORT_ACCOUNT_TOTALS: NamedQuery = NamedQuery {
    name: "report.account_totals",
    builtin: true,
    bql: "SELECT account, units(sum(position)) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, :type) AND date >= :from AND date <= :to
GROUP BY account
ORDER BY number(only(:currency, convert(sum(position), :currency, :to))), account",
    params: &[
        ("type", ParamType::Str),
        ("from", ParamType::Date),
        ("to", ParamType::Date),
        ("currency", ParamType::Str),
    ],
};

// ---- the plugin's own, in the same style ----

/// `spending` by month: `report.changes` for one subtree, each month valued at the prices of its last day in the range
pub const SPENDING_BY_MONTH: NamedQuery = NamedQuery {
    name: "mcp.spending_by_month",
    builtin: false,
    bql: "SELECT date_bin('month', date, 2001-01-01) AS month, units(sum(position)) AS units,
  convert(sum(position), :currency, least(max(date_bin('month', date, 2001-01-01)) + interval('month') - 1, :to)) AS value
WHERE under(account, :account) AND date >= :from AND date <= :to
GROUP BY month
ORDER BY month",
    params: &[
        ("account", ParamType::Str),
        ("from", ParamType::Date),
        ("to", ParamType::Date),
        ("currency", ParamType::Str),
    ],
};

/// `spending` by currency: what the subtree took in each currency, valued at the prices of `:to`
pub const SPENDING_BY_CURRENCY: NamedQuery = NamedQuery {
    name: "mcp.spending_by_currency",
    builtin: false,
    bql: "SELECT currency, sum(number) AS units, convert(sum(position), :currency, :to) AS value
WHERE under(account, :account) AND date >= :from AND date <= :to
GROUP BY currency
ORDER BY currency",
    params: &[
        ("account", ParamType::Str),
        ("from", ParamType::Date),
        ("to", ParamType::Date),
        ("currency", ParamType::Str),
    ],
};

/// `transactions`: one row per transaction, newest first, each filter left out when its parameter is NULL (as
/// `postings.matching` and `journals.page` do)
pub const TRANSACTIONS: NamedQuery = NamedQuery {
    name: "mcp.transactions",
    builtin: false,
    bql: "SELECT date, time, id, flag, payee, narration, tags, links, accounts
FROM #transactions
WHERE (:keyword IS NULL OR icontains(payee, :keyword) OR icontains(narration, :keyword))
  AND (:payee IS NULL OR payee = :payee)
  AND (:tags IS NULL OR intersects(tags, :tags))
  AND (:from IS NULL OR date >= :from)
  AND (:to IS NULL OR date <= :to)
ORDER BY seq DESC
LIMIT :limit OFFSET :offset",
    params: &[
        ("keyword", ParamType::Str),
        ("payee", ParamType::Str),
        ("tags", ParamType::Set),
        ("from", ParamType::Date),
        ("to", ParamType::Date),
        ("limit", ParamType::Int),
        ("offset", ParamType::Int),
    ],
};

/// every query of the catalogue
pub const QUERIES: [NamedQuery; 14] = [
    ACCOUNTS_LIST,
    ACCOUNTS_SUBTREE,
    ACCOUNTS_BALANCES,
    ACCOUNTS_SUBTREE_BALANCES,
    ACCOUNTS_JOURNAL_PAGE,
    JOURNALS_PAYEES,
    JOURNALS_ERRORS,
    COMMODITIES_LATEST_PRICES,
    COMMODITIES_LATEST_PRICE,
    REPORT_NET_WORTH,
    REPORT_ACCOUNT_TOTALS,
    SPENDING_BY_MONTH,
    SPENDING_BY_CURRENCY,
    TRANSACTIONS,
];

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use super::QUERIES;

    #[test]
    fn every_query_names_exactly_the_parameters_its_text_uses() {
        for query in QUERIES {
            let declared: BTreeSet<&str> = query.params.iter().map(|(name, _)| *name).collect();
            assert_eq!(declared.len(), query.params.len(), "{} declares a parameter twice", query.name);
            // every `:name` in the text, outside string literals
            let mut used = BTreeSet::new();
            let mut rest = query.bql;
            while let Some(at) = rest.find(':') {
                let after = &rest[at + 1..];
                let name: String = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !name.is_empty() && !name.chars().next().unwrap().is_ascii_digit() {
                    used.insert(name);
                }
                rest = after;
            }
            let used: BTreeSet<&str> = used.iter().map(String::as_str).collect();
            assert_eq!(used, declared, "{}: the parameters its text uses, and those it declares", query.name);
        }
        let names: BTreeSet<&str> = QUERIES.iter().map(|it| it.name).collect();
        assert_eq!(names.len(), QUERIES.len());
    }
}
