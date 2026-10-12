//! What the plugin needs from zhang, behind a trait: the dispatcher is unit tested natively against
//! [`FakeHost`], and the wasm build answers with the SDK ([`SdkHost`]).

use std::collections::BTreeMap;

use serde_json::Value;
/// A typed parameter value a query binds, as the `zhang_query_params` host function reads it (`{"type": "date",
/// "value": "2026-09-01"}`); `None` binds NULL, which leaves a NULL-able filter out.
pub use zhang_plugin_sdk::router::Param;
pub use zhang_plugin_sdk::router::{Column, LedgerInfo, QueryResult};
pub use zhang_plugin_sdk::{HostError, HostErrorKind};

/// the parameters of one query, by name (`from` for `:from`)
pub type Params = Vec<(String, Param)>;

/// the clock of the ledger, as `zhang_now` answers it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clock {
    /// RFC 3339, with the offset of the ledger's timezone
    pub now: String,
    /// `YYYY-MM-DD` in the ledger's timezone
    pub today: String,
    /// `HH:MM:SS` in the ledger's timezone
    pub time: String,
    /// an IANA name
    pub timezone: String,
}

/// The host functions the tools run on. Every call is read-only.
pub trait Host {
    /// run `bql` with `params` bound, with the limits of `POST /api/query`
    fn query(&self, bql: &str, params: &Params) -> Result<QueryResult, HostError>;
    /// the query schema: tables, columns and functions, as `GET /api/query/schema` returns it
    fn schema(&self) -> Result<Value, HostError>;
    fn ledger_info(&self) -> Result<LedgerInfo, HostError>;
    fn now(&self) -> Result<Clock, HostError>;
}

/// The host as the SDK reaches it inside zhang.
pub struct SdkHost;

impl Host for SdkHost {
    fn query(&self, bql: &str, params: &Params) -> Result<QueryResult, HostError> {
        if params.is_empty() {
            // `zhang_query`, which every zhang with router plugins has
            return zhang_plugin_sdk::router::query(bql);
        }
        let bound = params.iter().fold(zhang_plugin_sdk::router::Params::new(), |bound, (name, param)| {
            bound.bind(name.clone(), param.clone())
        });
        zhang_plugin_sdk::router::query_with_params(bql, &bound)
    }

    fn schema(&self) -> Result<Value, HostError> {
        zhang_plugin_sdk::router::schema()
    }

    fn ledger_info(&self) -> Result<LedgerInfo, HostError> {
        zhang_plugin_sdk::router::ledger_info()
    }

    fn now(&self) -> Result<Clock, HostError> {
        let now = zhang_plugin_sdk::clock::now()?;
        Ok(Clock {
            now: now.now.to_rfc3339(),
            today: now.today.format("%Y-%m-%d").to_string(),
            time: now.now.format("%H:%M:%S").to_string(),
            timezone: now.timezone,
        })
    }
}

/// A host for the unit tests: canned answers per BQL text, and a record of every query it ran.
#[derive(Default)]
pub struct FakeHost {
    /// the answer of each BQL text; a text without an answer is a `query` error at line 1, column 8
    pub answers: BTreeMap<String, Result<QueryResult, HostError>>,
    pub info: Option<LedgerInfo>,
    pub schema: Option<Value>,
    /// every query run, with its parameters, in order
    pub ran: std::cell::RefCell<Vec<(String, Params)>>,
}

impl FakeHost {
    /// a host of a CNY ledger titled "Home" at noon on 2024-03-16 in Asia/Shanghai, with no canned queries
    pub fn new() -> FakeHost {
        FakeHost {
            info: Some(LedgerInfo {
                title: Some("Home".to_owned()),
                operating_currency: "CNY".to_owned(),
                timezone: "Asia/Shanghai".to_owned(),
            }),
            schema: Some(serde_json::json!({"columns": [], "functions": [], "tables": [], "keywords": []})),
            ..FakeHost::default()
        }
    }

    /// this host answering `bql` with `columns` and `rows`
    pub fn answering(mut self, bql: &str, columns: &[(&str, &str)], rows: Vec<Vec<Value>>) -> FakeHost {
        let columns = columns
            .iter()
            .map(|(name, ty)| Column {
                name: (*name).to_owned(),
                ty: (*ty).to_owned(),
            })
            .collect();
        self.answers.insert(bql.to_owned(), Ok(QueryResult { columns, rows }));
        self
    }

    /// this host failing `bql` with `error`
    pub fn failing(mut self, bql: &str, error: HostError) -> FakeHost {
        self.answers.insert(bql.to_owned(), Err(error));
        self
    }

    /// the parameters of the last query run
    pub fn last_params(&self) -> Params {
        self.ran.borrow().last().map(|(_, params)| params.clone()).unwrap_or_default()
    }
}

impl Host for FakeHost {
    fn query(&self, bql: &str, params: &Params) -> Result<QueryResult, HostError> {
        self.ran.borrow_mut().push((bql.to_owned(), params.clone()));
        match self.answers.get(bql) {
            Some(answer) => answer.clone(),
            None => Err(HostError {
                kind: HostErrorKind::Query,
                message: format!("unknown column `nope` in {}", bql.lines().next().unwrap_or_default()),
                line: Some(1),
                column: Some(8),
            }),
        }
    }

    fn schema(&self) -> Result<Value, HostError> {
        self.schema.clone().ok_or_else(|| HostError::new(HostErrorKind::Unavailable, "no schema"))
    }

    fn ledger_info(&self) -> Result<LedgerInfo, HostError> {
        self.info.clone().ok_or_else(|| HostError::new(HostErrorKind::Unavailable, "no ledger"))
    }

    fn now(&self) -> Result<Clock, HostError> {
        Ok(Clock {
            now: "2024-03-16T12:00:00+08:00".to_owned(),
            today: "2024-03-16".to_owned(),
            time: "12:00:00".to_owned(),
            timezone: "Asia/Shanghai".to_owned(),
        })
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::Param;

    #[test]
    fn params_serialize_as_the_host_function_reads_them() {
        assert_eq!(serde_json::to_value(Param::Date(None)).unwrap(), json!({"type": "date", "value": null}));
        assert_eq!(
            serde_json::to_value(Param::Set(Some(vec!["a".to_owned()]))).unwrap(),
            json!({"type": "set", "value": ["a"]})
        );
        assert_eq!(serde_json::to_value(Param::Int(Some(3))).unwrap(), json!({"type": "int", "value": 3}));
    }
}
