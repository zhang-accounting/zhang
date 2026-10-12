//! The tool catalogue: twelve read-only tools, each a mapping of its arguments onto one BQL query (or one host
//! function) and of the rows onto `structuredContent`. The figures themselves are the query engine's.

pub mod args;
pub mod queries;
pub mod shape;

use serde_json::{json, Map, Value};

use self::args::{input_schema, Arg, Args, Kind};
pub use self::queries::QUERIES;
use self::queries::{NamedQuery, ParamType};
use self::shape::ToolError;
use crate::config::McpConfig;
use crate::host::{Host, Param, Params};
use crate::jsonrpc::RpcError;

/// the annotations of every tool: read-only, idempotent, closed world
const ANNOTATIONS: &str = r#"{"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}"#;

/// what a tool runs with
pub struct Call<'a> {
    pub args: Args,
    pub host: &'a dyn Host,
    pub config: &'a McpConfig,
}

impl Call<'_> {
    /// the ledger's operating currency, or the `currency` argument when the call gives one
    fn currency(&self) -> Result<String, ToolError> {
        match self.args.str("currency") {
            Some(currency) => Ok(currency.to_owned()),
            None => Ok(self.host.ledger_info()?.operating_currency),
        }
    }

    /// `limit` as this call runs with it
    fn limit(&self) -> usize {
        self.config.limit(self.args.int("limit").map(|it| it as usize))
    }

    /// what a query with `LIMIT :limit` is asked for: one row more than [`Call::limit`], so that `truncated` can say
    /// whether more rows exist after the plugin cuts the result at `limit`
    fn fetch(&self) -> i64 {
        self.limit() as i64 + 1
    }

    fn offset(&self) -> i64 {
        self.args.int("offset").unwrap_or(0)
    }

    /// run `query` with `params` bound and shape the rows, at most `limit` of them
    fn rows(&self, query: &NamedQuery, params: Params, limit: usize) -> Result<Value, ToolError> {
        debug_assert!(
            params.len() == query.params.len()
                && params
                    .iter()
                    .all(|(name, param)| query.params.iter().any(|(declared, ty)| declared == name && matches(ty, param)))
        );
        Ok(shape::rows(self.host.query(query.bql, &params)?, limit))
    }
}

/// whether `param` is of the declared type
fn matches(ty: &ParamType, param: &Param) -> bool {
    matches!(
        (ty, param),
        (ParamType::Int, Param::Int(_)) | (ParamType::Str, Param::Str(_)) | (ParamType::Date, Param::Date(_)) | (ParamType::Set, Param::Set(_))
    )
}

fn text(value: &str) -> Param {
    Param::Str(Some(value.to_owned()))
}

fn opt_text(value: Option<&str>) -> Param {
    Param::Str(value.map(str::to_owned))
}

fn date(value: &str) -> Param {
    Param::Date(Some(value.to_owned()))
}

fn opt_date(value: Option<&str>) -> Param {
    Param::Date(value.map(str::to_owned))
}

fn int(value: i64) -> Param {
    Param::Int(Some(value))
}

fn bind(pairs: Vec<(&str, Param)>) -> Params {
    pairs.into_iter().map(|(name, param)| (name.to_owned(), param)).collect()
}

/// one tool of the catalogue
pub struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub args: &'static [Arg],
    pub output_schema: fn() -> Value,
    pub run: fn(&Call<'_>) -> Result<Value, ToolError>,
}

impl Tool {
    /// the tool as `tools/list` describes it
    pub fn describe(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": input_schema(self.args),
            "outputSchema": (self.output_schema)(),
            "annotations": serde_json::from_str::<Value>(ANNOTATIONS).expect("constant JSON"),
        })
    }
}

const LIMIT: Arg = Arg::optional(
    "limit",
    Kind::Int { min: 1 },
    "the most rows to return; default 200, capped by the server. `truncated` tells when more exist",
);
const OFFSET: Arg = Arg::optional("offset", Kind::Int { min: 0 }, "rows to skip, for paging; default 0");
const FROM: Arg = Arg::required("from", Kind::Date, "the first day of the range, YYYY-MM-DD");
const TO: Arg = Arg::required("to", Kind::Date, "the last day of the range, YYYY-MM-DD, included");
const CURRENCY: Arg = Arg::optional("currency", Kind::Str, "the currency to value in; default the ledger's operating currency");

/// the twelve tools, in the order `tools/list` returns them
pub static TOOLS: [Tool; 12] = [
    Tool {
        name: "ledger_info",
        title: "Ledger information",
        description: "The ledger's title, operating currency and timezone, and the current date and time by its clock. \
                      Call it first to know today's date before asking for a period.",
        args: &[],
        output_schema: shape::ledger_info_schema,
        run: |call| {
            let info = call.host.ledger_info()?;
            let clock = call.host.now()?;
            Ok(json!({
                "title": info.title,
                "operating_currency": info.operating_currency,
                "timezone": info.timezone,
                "now": clock.now,
                "today": clock.today,
            }))
        },
    },
    Tool {
        name: "schema",
        title: "Query schema",
        description: "The tables, columns and functions of BQL, the ledger's query language, as the `query` tool runs it. \
                      Read it before writing a query.",
        args: &[],
        output_schema: shape::schema_schema,
        run: |call| Ok(call.host.schema()?),
    },
    Tool {
        name: "query",
        title: "Run BQL",
        description: "Run a read-only BQL query over the ledger and return its rows. BQL is SQL-like: `SELECT account, \
                      sum(position) WHERE under(account, 'Expenses') AND date >= 2026-01-01 GROUP BY account`; the default \
                      table is the postings, `FROM #transactions`, `#accounts`, `#prices`, `#errors` read the others. A \
                      failing query comes back as an error naming the line and column. Call `schema` for the columns and \
                      functions.",
        args: &[Arg::required("bql", Kind::Str, "the query text"), LIMIT],
        output_schema: shape::rows_schema,
        run: |call| {
            let bql = call.args.str("bql").unwrap_or_default();
            Ok(shape::rows(call.host.query(bql, &Params::new())?, call.limit()))
        },
    },
    Tool {
        name: "accounts",
        title: "Accounts",
        description: "The accounts of the ledger with their open and close dates, alias and status now. Give `prefix` for \
                      one account and its sub-accounts (`Expenses:Food`), and `status` to see closed accounts too.",
        args: &[
            Arg::optional("prefix", Kind::Str, "an account; it and its sub-accounts are listed. Absent: every account"),
            Arg::optional(
                "status",
                Kind::Enum(&["open", "closed", "all"]),
                "which accounts to list by their status now; default open",
            ),
        ],
        output_schema: shape::rows_schema,
        run: |call| {
            let clock = call.host.now()?;
            let at = vec![("date", date(&clock.today)), ("time", text(&clock.time))];
            let result = match call.args.str("prefix") {
                Some(prefix) => call
                    .host
                    .query(queries::ACCOUNTS_SUBTREE.bql, &bind([vec![("account", text(prefix))], at].concat()))?,
                None => call.host.query(queries::ACCOUNTS_LIST.bql, &bind(at))?,
            };
            let status = call.args.str_or("status", "open");
            let result = match (status, result.column("status")) {
                ("all", _) | (_, None) => result,
                (status, Some(column)) => crate::host::QueryResult {
                    rows: result
                        .rows
                        .into_iter()
                        .filter(|row| row.get(column).and_then(Value::as_str) == Some(status))
                        .collect(),
                    ..result
                },
            };
            Ok(shape::rows(result, call.config.max_rows))
        },
    },
    Tool {
        name: "balances",
        title: "Balances",
        description: "The balance of every account per currency: the units held, their value in the operating currency \
                      at today's prices, and the date of the first posting. Give `account` for one account and its \
                      sub-accounts.",
        args: &[Arg::optional(
            "account",
            Kind::Str,
            "an account; it and its sub-accounts are listed. Absent: every account",
        )],
        output_schema: shape::rows_schema,
        run: |call| {
            let currency = call.host.ledger_info()?.operating_currency;
            match call.args.str("account") {
                Some(account) => call.rows(
                    &queries::ACCOUNTS_SUBTREE_BALANCES,
                    bind(vec![("account", text(account)), ("operating_currency", text(&currency))]),
                    call.config.max_rows,
                ),
                None => call.rows(
                    &queries::ACCOUNTS_BALANCES,
                    bind(vec![("operating_currency", text(&currency))]),
                    call.config.max_rows,
                ),
            }
        },
    },
    Tool {
        name: "net_worth",
        title: "Net worth",
        description: "The net worth, the balance of the Assets and Liabilities accounts, at the end of a day: `units` lists what \
                      is held per currency, `value` is the total valued in one currency at the prices of that day. This is the \
                      figure the app's report shows.",
        args: &[Arg::optional("date", Kind::Date, "the day, YYYY-MM-DD; default today"), CURRENCY],
        output_schema: shape::rows_schema,
        run: |call| {
            let currency = call.currency()?;
            let to = match call.args.str("date") {
                Some(date) => date.to_owned(),
                None => call.host.now()?.today,
            };
            call.rows(
                &queries::REPORT_NET_WORTH,
                bind(vec![("to", date(&to)), ("currency", text(&currency))]),
                call.config.max_rows,
            )
        },
    },
    Tool {
        name: "spending",
        title: "Spending",
        description: "What an account and its sub-accounts took in a period (default: all of Expenses), grouped by account, by \
                      month or by currency: `units` per currency and `value` in one currency at the prices of the period's \
                      last day. Positive numbers are expenses; for Income accounts they are negative.",
        args: &[
            Arg::optional("account", Kind::Str, "the account whose subtree to sum; default Expenses"),
            FROM,
            TO,
            Arg::optional(
                "group_by",
                Kind::Enum(&["account", "month", "currency"]),
                "how to group the rows; default account",
            ),
            CURRENCY,
        ],
        output_schema: shape::rows_schema,
        run: |call| {
            let currency = call.currency()?;
            let account = call.args.str_or("account", "Expenses");
            let (from, to) = (call.args.str("from").unwrap_or_default(), call.args.str("to").unwrap_or_default());
            let range = vec![("from", date(from)), ("to", date(to)), ("currency", text(&currency))];
            let (query, params) = match call.args.str_or("group_by", "account") {
                "month" => (&queries::SPENDING_BY_MONTH, [vec![("account", text(account))], range].concat()),
                "currency" => (&queries::SPENDING_BY_CURRENCY, [vec![("account", text(account))], range].concat()),
                _ => (&queries::REPORT_ACCOUNT_TOTALS, [vec![("type", text(account))], range].concat()),
            };
            call.rows(query, bind(params), call.config.max_rows)
        },
    },
    Tool {
        name: "journal",
        title: "Account journal",
        description: "The postings of an account and its sub-accounts in ledger order, one row per posting with the running \
                      balance after it. Page with `limit` and `offset`.",
        args: &[Arg::required("account", Kind::Str, "the account; its sub-accounts are included"), LIMIT, OFFSET],
        output_schema: shape::rows_schema,
        run: |call| {
            let limit = call.limit();
            let account = call.args.str("account").unwrap_or_default();
            call.rows(
                &queries::ACCOUNTS_JOURNAL_PAGE,
                bind(vec![("account", text(account)), ("limit", int(call.fetch())), ("offset", int(call.offset()))]),
                limit,
            )
        },
    },
    Tool {
        name: "transactions",
        title: "Find transactions",
        description: "The transactions matching a keyword (in the payee or narration, case-insensitive), an exact payee, any of \
                      some tags and a date range, newest first, with the accounts each one posts to. Every filter is optional. \
                      `payees` lists the exact payees; `journal` shows the postings of one account.",
        args: &[
            Arg::optional("keyword", Kind::Str, "text to find in the payee or the narration"),
            Arg::optional("payee", Kind::Str, "the exact payee"),
            Arg::optional("tags", Kind::StrList, "tags, without #; a transaction matches when it has any of them"),
            Arg::optional("from", Kind::Date, "the first day, YYYY-MM-DD"),
            Arg::optional("to", Kind::Date, "the last day, YYYY-MM-DD, included"),
            LIMIT,
            OFFSET,
        ],
        output_schema: shape::rows_schema,
        run: |call| {
            let limit = call.limit();
            let tags = call.args.list("tags").filter(|tags| !tags.is_empty());
            call.rows(
                &queries::TRANSACTIONS,
                bind(vec![
                    ("keyword", opt_text(call.args.str("keyword"))),
                    ("payee", opt_text(call.args.str("payee"))),
                    ("tags", Param::Set(tags)),
                    ("from", opt_date(call.args.str("from"))),
                    ("to", opt_date(call.args.str("to"))),
                    ("limit", int(call.fetch())),
                    ("offset", int(call.offset())),
                ]),
                limit,
            )
        },
    },
    Tool {
        name: "payees",
        title: "Payees",
        description: "Every payee of the ledger's transactions, once and sorted. Use it to find the exact payee for `transactions`.",
        args: &[],
        output_schema: shape::rows_schema,
        run: |call| call.rows(&queries::JOURNALS_PAYEES, Params::new(), call.config.max_rows),
    },
    Tool {
        name: "errors",
        title: "Ledger errors",
        description: "The problems zhang found in the ledger (unbalanced transactions, failed balance assertions, unknown \
                      accounts, …), by file and position, with the source text of each. Page with `limit` and `offset`.",
        args: &[LIMIT, OFFSET],
        output_schema: shape::rows_schema,
        run: |call| {
            let limit = call.limit();
            call.rows(
                &queries::JOURNALS_ERRORS,
                bind(vec![("size", int(call.fetch())), ("offset", int(call.offset()))]),
                limit,
            )
        },
    },
    Tool {
        name: "prices",
        title: "Latest prices",
        description: "The latest price of each commodity in a currency as of today, with its date: the rate the valuations use. \
                      Give `commodity` for one commodity.",
        args: &[
            Arg::optional("commodity", Kind::Str, "one commodity, e.g. USD or AAPL; absent: every commodity with a price"),
            CURRENCY,
        ],
        output_schema: shape::rows_schema,
        run: |call| {
            let currency = call.currency()?;
            match call.args.str("commodity") {
                Some(commodity) => call.rows(
                    &queries::COMMODITIES_LATEST_PRICE,
                    bind(vec![("commodity", text(commodity)), ("currency", text(&currency))]),
                    call.config.max_rows,
                ),
                None => call.rows(
                    &queries::COMMODITIES_LATEST_PRICES,
                    bind(vec![("currency", text(&currency))]),
                    call.config.max_rows,
                ),
            }
        },
    },
];

/// the tool named `name`
pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// `tools/list`: every tool as the clients read it
pub fn list() -> Vec<Value> {
    TOOLS.iter().map(Tool::describe).collect()
}

/// `tools/call`: the `CallToolResult` of running the tool `params.name` with `params.arguments`. An unknown tool or
/// arguments that do not fit the tool's schema are protocol errors (`-32602`); a failing query is a tool execution
/// error (`isError: true`) inside the `Ok`.
pub fn call(params: &Map<String, Value>, host: &dyn Host, config: &McpConfig) -> Result<Value, RpcError> {
    let name = match params.get("name") {
        Some(Value::String(name)) => name.as_str(),
        _ => return Err(RpcError::invalid_params("params.name is required: the name of the tool to call")),
    };
    let tool = find(name).ok_or_else(|| RpcError::invalid_params(format!("Unknown tool: {name}")))?;
    let empty = Map::new();
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(arguments)) => arguments,
        Some(_) => return Err(RpcError::invalid_params("params.arguments must be an object")),
    };
    let args = Args::validate(tool.args, arguments)?;
    let call = Call { args, host, config };
    Ok(match (tool.run)(&call) {
        Ok(structured) => shape::success(structured),
        Err(error) => shape::failure(&error),
    })
}

#[cfg(test)]
mod test {
    use std::collections::BTreeSet;

    use serde_json::{json, Map, Value};

    use super::{call, list, queries, TOOLS};
    use crate::config::McpConfig;
    use crate::host::{FakeHost, HostError, HostErrorKind, Param};

    fn run(host: &FakeHost, name: &str, arguments: Value) -> Result<Value, String> {
        let params: Map<String, Value> = json!({"name": name, "arguments": arguments}).as_object().unwrap().clone();
        call(&params, host, &McpConfig::default()).map_err(|e| format!("{}: {}", e.code, e.message))
    }

    fn structured(host: &FakeHost, name: &str, arguments: Value) -> Value {
        let result = run(host, name, arguments).unwrap();
        assert!(result.get("isError").is_none(), "{result}");
        let text: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"], "the text block is the structured content");
        result["structuredContent"].clone()
    }

    fn names(params: &[(String, Param)]) -> Vec<&str> {
        params.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn the_catalogue_has_twelve_well_formed_tools() {
        let listed = list();
        assert_eq!(listed.len(), 12);
        let names: Vec<&str> = listed.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "ledger_info",
                "schema",
                "query",
                "accounts",
                "balances",
                "net_worth",
                "spending",
                "journal",
                "transactions",
                "payees",
                "errors",
                "prices"
            ]
        );
        assert_eq!(names.iter().collect::<BTreeSet<_>>().len(), 12);
        for tool in &listed {
            let name = tool["name"].as_str().unwrap();
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{name}");
            assert!(!tool["description"].as_str().unwrap().is_empty(), "{name}");
            let input = &tool["inputSchema"];
            assert_eq!(input["type"], "object", "{name}");
            assert_eq!(input["additionalProperties"], false, "{name}");
            if let Some(properties) = input.get("properties") {
                assert!(properties.is_object() && !properties.as_object().unwrap().is_empty(), "{name}");
                for (arg, schema) in properties.as_object().unwrap() {
                    assert!(schema["type"].is_string(), "{name}.{arg}");
                    assert!(schema["description"].is_string(), "{name}.{arg}");
                }
            }
            if let Some(required) = input.get("required") {
                for arg in required.as_array().unwrap() {
                    assert!(
                        input["properties"].get(arg.as_str().unwrap()).is_some(),
                        "{name} requires an argument it does not declare"
                    );
                }
            }
            assert_eq!(tool["outputSchema"]["type"], "object", "{name}");
            assert_eq!(
                tool["annotations"],
                json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false})
            );
        }
    }

    #[test]
    fn every_query_of_the_catalogue_is_run_by_a_tool() {
        // the fake host records the BQL of every call; run each tool in every branch and collect the texts
        let host = FakeHost::new();
        let today = "2024-03-16";
        for (name, arguments) in [
            ("accounts", json!({})),
            ("accounts", json!({"prefix": "Assets"})),
            ("balances", json!({})),
            ("balances", json!({"account": "Assets"})),
            ("net_worth", json!({})),
            ("spending", json!({"from": today, "to": today})),
            ("spending", json!({"from": today, "to": today, "group_by": "month"})),
            ("spending", json!({"from": today, "to": today, "group_by": "currency"})),
            ("journal", json!({"account": "Assets"})),
            ("transactions", json!({})),
            ("payees", json!({})),
            ("errors", json!({})),
            ("prices", json!({})),
            ("prices", json!({"commodity": "USD"})),
        ] {
            run(&host, name, arguments).unwrap();
        }
        let ran: BTreeSet<String> = host.ran.borrow().iter().map(|(bql, _)| bql.clone()).collect();
        let catalogue: BTreeSet<String> = queries::QUERIES.iter().map(|it| it.bql.to_owned()).collect();
        assert_eq!(ran, catalogue);
    }

    #[test]
    fn protocol_errors_for_unknown_tools_and_bad_arguments() {
        let host = FakeHost::new();
        assert_eq!(run(&host, "nope", json!({})).unwrap_err(), "-32602: Unknown tool: nope");
        assert_eq!(run(&host, "journal", json!({})).unwrap_err(), "-32602: the argument `account` is required");
        assert_eq!(
            run(&host, "query", json!({"bql": "SELECT 1", "limit": "ten"})).unwrap_err(),
            "-32602: the argument `limit` must be an integer of at least 1"
        );
        assert_eq!(run(&host, "payees", json!({"x": 1})).unwrap_err(), "-32602: unknown argument `x`");
        assert_eq!(
            run(&host, "spending", json!({"from": "2024-01-01", "to": "2024-01-31", "group_by": "week"})).unwrap_err(),
            "-32602: the argument `group_by` must be one of `account`, `month`, `currency`"
        );
        let no_name: Map<String, Value> = json!({"arguments": {}}).as_object().unwrap().clone();
        assert_eq!(call(&no_name, &host, &McpConfig::default()).unwrap_err().code, -32602);
        let bad_arguments: Map<String, Value> = json!({"name": "payees", "arguments": [1]}).as_object().unwrap().clone();
        assert_eq!(call(&bad_arguments, &host, &McpConfig::default()).unwrap_err().code, -32602);
        // arguments may be absent
        let no_arguments: Map<String, Value> = json!({"name": "ledger_info"}).as_object().unwrap().clone();
        assert!(call(&no_arguments, &host, &McpConfig::default()).is_ok());
    }

    #[test]
    fn query_errors_are_tool_execution_errors_without_structured_content() {
        let host = FakeHost::new().failing(
            "SELECT nope",
            HostError {
                kind: HostErrorKind::Query,
                message: "unknown column `nope`".to_owned(),
                line: Some(1),
                column: Some(8),
            },
        );
        let result = run(&host, "query", json!({"bql": "SELECT nope"})).unwrap();
        assert_eq!(
            result,
            json!({"content": [{"type": "text", "text": "query error at line 1, column 8: unknown column `nope`"}], "isError": true})
        );
        let unavailable = FakeHost { info: None, ..FakeHost::new() };
        let result = run(&unavailable, "ledger_info", json!({})).unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("the ledger is not available to this tool"));
    }

    #[test]
    fn ledger_info_and_schema() {
        let host = FakeHost::new();
        assert_eq!(
            structured(&host, "ledger_info", json!({})),
            json!({"title": "Home", "operating_currency": "CNY", "timezone": "Asia/Shanghai", "now": "2024-03-16T12:00:00+08:00", "today": "2024-03-16"})
        );
        assert_eq!(
            structured(&host, "schema", json!({})),
            json!({"columns": [], "functions": [], "tables": [], "keywords": []})
        );
    }

    #[test]
    fn query_truncates_at_the_limit_and_the_cap() {
        let rows: Vec<Vec<Value>> = (0..300).map(|i| vec![json!(i)]).collect();
        let host = FakeHost::new().answering("SELECT n", &[("n", "int")], rows);
        let all = structured(&host, "query", json!({"bql": "SELECT n"}));
        assert_eq!((all["row_count"].as_u64(), all["truncated"].as_bool()), (Some(200), Some(true)));
        assert_eq!(all["columns"], json!([{"name": "n", "type": "int"}]));
        let few = structured(&host, "query", json!({"bql": "SELECT n", "limit": 3}));
        assert_eq!(few["rows"], json!([[0], [1], [2]]));
        let plenty = structured(&host, "query", json!({"bql": "SELECT n", "limit": 5000}));
        assert_eq!((plenty["row_count"].as_u64(), plenty["truncated"].as_bool()), (Some(300), Some(false)));
        assert_eq!(host.last_params(), vec![], "the query tool binds nothing");

        let capped = McpConfig {
            max_rows: 10,
            ..McpConfig::default()
        };
        let params: Map<String, Value> = json!({"name": "query", "arguments": {"bql": "SELECT n", "limit": 50}})
            .as_object()
            .unwrap()
            .clone();
        let result = call(&params, &host, &capped).unwrap();
        assert_eq!(result["structuredContent"]["row_count"], 10);
        assert_eq!(result["structuredContent"]["truncated"], true);
    }

    #[test]
    fn accounts_binds_the_clock_and_filters_by_status() {
        let rows = vec![
            vec![json!("Assets:Cash"), json!(null), json!(null), json!(null), json!("open")],
            vec![json!("Assets:Old"), json!(null), json!(null), json!(null), json!("closed")],
        ];
        let columns = [("account", "str"), ("open", "str"), ("close", "str"), ("alias", "str"), ("status", "str")];
        let host = FakeHost::new()
            .answering(queries::ACCOUNTS_LIST.bql, &columns, rows.clone())
            .answering(queries::ACCOUNTS_SUBTREE.bql, &columns, rows);
        let open = structured(&host, "accounts", json!({}));
        assert_eq!(open["rows"].as_array().unwrap().len(), 1);
        assert_eq!(open["rows"][0][0], "Assets:Cash");
        assert_eq!(
            host.last_params(),
            vec![
                ("date".to_owned(), Param::Date(Some("2024-03-16".to_owned()))),
                ("time".to_owned(), Param::Str(Some("12:00:00".to_owned()))),
            ]
        );
        let closed = structured(&host, "accounts", json!({"status": "closed", "prefix": "Assets"}));
        assert_eq!(closed["rows"][0][0], "Assets:Old");
        assert_eq!(names(&host.last_params()), ["account", "date", "time"]);
        assert_eq!(host.last_params()[0].1, Param::Str(Some("Assets".to_owned())));
        let all = structured(&host, "accounts", json!({"status": "all"}));
        assert_eq!(all["row_count"], 2);
    }

    #[test]
    fn balances_and_net_worth_bind_the_operating_currency() {
        let host = FakeHost::new()
            .answering(queries::ACCOUNTS_BALANCES.bql, &[("account", "str")], vec![])
            .answering(queries::ACCOUNTS_SUBTREE_BALANCES.bql, &[("account", "str")], vec![])
            .answering(
                queries::REPORT_NET_WORTH.bql,
                &[("value", "amount")],
                vec![vec![json!({"number": "10", "currency": "CNY"})]],
            );
        structured(&host, "balances", json!({}));
        assert_eq!(host.last_params(), vec![("operating_currency".to_owned(), Param::Str(Some("CNY".to_owned())))]);
        structured(&host, "balances", json!({"account": "Assets:Bank"}));
        assert_eq!(names(&host.last_params()), ["account", "operating_currency"]);
        let worth = structured(&host, "net_worth", json!({}));
        assert_eq!(worth["rows"][0][0]["number"], "10");
        assert_eq!(
            host.last_params(),
            vec![
                ("to".to_owned(), Param::Date(Some("2024-03-16".to_owned()))),
                ("currency".to_owned(), Param::Str(Some("CNY".to_owned()))),
            ]
        );
        structured(&host, "net_worth", json!({"date": "2023-12-31", "currency": "USD"}));
        assert_eq!(
            host.last_params(),
            vec![
                ("to".to_owned(), Param::Date(Some("2023-12-31".to_owned()))),
                ("currency".to_owned(), Param::Str(Some("USD".to_owned()))),
            ]
        );
    }

    #[test]
    fn spending_picks_the_query_by_grouping() {
        let host = FakeHost::new()
            .answering(queries::REPORT_ACCOUNT_TOTALS.bql, &[("account", "str")], vec![])
            .answering(queries::SPENDING_BY_MONTH.bql, &[("month", "date")], vec![])
            .answering(queries::SPENDING_BY_CURRENCY.bql, &[("currency", "str")], vec![]);
        structured(&host, "spending", json!({"from": "2024-01-01", "to": "2024-01-31"}));
        assert_eq!(
            host.last_params(),
            vec![
                ("type".to_owned(), Param::Str(Some("Expenses".to_owned()))),
                ("from".to_owned(), Param::Date(Some("2024-01-01".to_owned()))),
                ("to".to_owned(), Param::Date(Some("2024-01-31".to_owned()))),
                ("currency".to_owned(), Param::Str(Some("CNY".to_owned()))),
            ]
        );
        structured(
            &host,
            "spending",
            json!({"account": "Expenses:Food", "from": "2024-01-01", "to": "2024-03-31", "group_by": "month"}),
        );
        assert_eq!(host.ran.borrow().last().unwrap().0, queries::SPENDING_BY_MONTH.bql);
        assert_eq!(host.last_params()[0], ("account".to_owned(), Param::Str(Some("Expenses:Food".to_owned()))));
        structured(
            &host,
            "spending",
            json!({"from": "2024-01-01", "to": "2024-01-31", "group_by": "currency", "currency": "USD"}),
        );
        assert_eq!(host.ran.borrow().last().unwrap().0, queries::SPENDING_BY_CURRENCY.bql);
        assert_eq!(host.last_params()[3], ("currency".to_owned(), Param::Str(Some("USD".to_owned()))));
    }

    #[test]
    fn paged_tools_bind_limit_and_offset() {
        let host = FakeHost::new()
            .answering(queries::ACCOUNTS_JOURNAL_PAGE.bql, &[("date", "date")], vec![])
            .answering(queries::JOURNALS_ERRORS.bql, &[("id", "str")], vec![])
            .answering(queries::TRANSACTIONS.bql, &[("date", "date")], vec![]);
        structured(&host, "journal", json!({"account": "Assets", "limit": 5, "offset": 10}));
        assert_eq!(
            host.last_params(),
            vec![
                ("account".to_owned(), Param::Str(Some("Assets".to_owned()))),
                ("limit".to_owned(), Param::Int(Some(6))),
                ("offset".to_owned(), Param::Int(Some(10))),
            ]
        );
        structured(&host, "errors", json!({}));
        assert_eq!(
            host.last_params(),
            vec![("size".to_owned(), Param::Int(Some(201))), ("offset".to_owned(), Param::Int(Some(0)))]
        );
        structured(
            &host,
            "transactions",
            json!({"keyword": "cafe", "tags": ["trip"], "from": "2024-01-01", "limit": 20}),
        );
        assert_eq!(
            host.last_params(),
            vec![
                ("keyword".to_owned(), Param::Str(Some("cafe".to_owned()))),
                ("payee".to_owned(), Param::Str(None)),
                ("tags".to_owned(), Param::Set(Some(vec!["trip".to_owned()]))),
                ("from".to_owned(), Param::Date(Some("2024-01-01".to_owned()))),
                ("to".to_owned(), Param::Date(None)),
                ("limit".to_owned(), Param::Int(Some(21))),
                ("offset".to_owned(), Param::Int(Some(0))),
            ]
        );
        // an empty tag list is no filter
        structured(&host, "transactions", json!({"tags": []}));
        assert_eq!(host.last_params()[2], ("tags".to_owned(), Param::Set(None)));
        // the cap applies to the bound limit too
        let capped = McpConfig {
            max_rows: 7,
            ..McpConfig::default()
        };
        let params: Map<String, Value> = json!({"name": "journal", "arguments": {"account": "Assets", "limit": 100}})
            .as_object()
            .unwrap()
            .clone();
        call(&params, &host, &capped).unwrap();
        assert_eq!(host.last_params()[1], ("limit".to_owned(), Param::Int(Some(8))));
        // the extra row only tells that more exist: the result is cut at the limit
        let rows: Vec<Vec<Value>> = (0..8).map(|i| vec![json!(i)]).collect();
        let host = FakeHost::new().answering(queries::ACCOUNTS_JOURNAL_PAGE.bql, &[("n", "int")], rows);
        let result = call(&params, &host, &capped).unwrap();
        assert_eq!(result["structuredContent"]["row_count"], 7);
        assert_eq!(result["structuredContent"]["truncated"], true);
    }

    #[test]
    fn prices_default_to_the_operating_currency() {
        let host = FakeHost::new()
            .answering(queries::COMMODITIES_LATEST_PRICES.bql, &[("currency", "str")], vec![])
            .answering(queries::COMMODITIES_LATEST_PRICE.bql, &[("currency", "str")], vec![])
            .answering(queries::JOURNALS_PAYEES.bql, &[("payee", "str")], vec![]);
        structured(&host, "prices", json!({}));
        assert_eq!(host.last_params(), vec![("currency".to_owned(), Param::Str(Some("CNY".to_owned())))]);
        structured(&host, "prices", json!({"commodity": "AAPL", "currency": "USD"}));
        assert_eq!(
            host.last_params(),
            vec![
                ("commodity".to_owned(), Param::Str(Some("AAPL".to_owned()))),
                ("currency".to_owned(), Param::Str(Some("USD".to_owned()))),
            ]
        );
        structured(&host, "payees", json!({}));
        assert_eq!(host.last_params(), vec![]);
    }

    #[test]
    fn the_catalogue_constants_agree_with_the_tools() {
        assert_eq!(TOOLS.len(), 12);
        assert_eq!(queries::QUERIES.iter().filter(|it| it.builtin).count(), 11);
    }
}
