//! `summary`, an example router plugin built with `zhang-plugin-sdk`. Declared with
//!
//! ```zhang
//! option "features.plugin" "true"
//! plugin "plugins/summary.wasm"
//! ```
//!
//! it serves, behind zhang's authentication:
//!
//! - `GET /api/plugins/summary/` an HTML page of every account's balance;
//! - `GET /api/plugins/summary/balances` the same balances as JSON, for scripts;
//! - `404` for any other path, and `405` for any other method.

use zhang_plugin_sdk::router::{self, Request, Response};
use zhang_plugin_sdk::serde_json::{json, Value};
use zhang_plugin_sdk::{clock, plugin, Error};

plugin! {
    name: "summary",
    version: env!("CARGO_PKG_VERSION"),
    router: route,
}

/// the balance of every account, as `zhang_query` runs it over the ledger
const BALANCES: &str = "SELECT account, sum(position) AS balance GROUP BY account ORDER BY account";

fn route(request: Request) -> Result<Response, Error> {
    if request.method != "GET" {
        return Ok(Response::text("only GET is supported").with_status(405).with_header("allow", "GET"));
    }
    match request.path.as_str() {
        "/" => page(),
        "/balances" => Response::json(&balances()?),
        _ => Ok(Response::text(format!("no page at {}", request.path)).with_status(404)),
    }
}

/// every account with its balance: `[{"account": "Assets:Cash", "balance": "-10 CNY"}]`
fn balances() -> Result<Vec<Value>, Error> {
    let result = router::query(BALANCES)?;
    let (Some(account), Some(balance)) = (result.column("account"), result.column("balance")) else {
        return Err(Error::msg("the balance query has no account or balance column"));
    };
    Ok(result
        .rows
        .iter()
        .map(|row| json!({"account": row.get(account), "balance": row.get(balance).map(inventory_text)}))
        .collect())
}

/// an inventory cell as text, e.g. `-10 CNY, 2 AAPL`
fn inventory_text(cell: &Value) -> String {
    let positions = cell["positions"].as_array().map(Vec::as_slice).unwrap_or_default();
    let amounts: Vec<String> = positions
        .iter()
        .map(|position| {
            let units = &position["units"];
            format!("{} {}", units["number"].as_str().unwrap_or("0"), units["currency"].as_str().unwrap_or_default())
        })
        .collect();
    amounts.join(", ")
}

fn page() -> Result<Response, Error> {
    let info = router::ledger_info()?;
    let today = clock::today()?;
    let rows: String = balances()?
        .iter()
        .map(|row| {
            format!(
                "<tr><td>{}</td><td>{}</td></tr>",
                escape(row["account"].as_str().unwrap_or_default()),
                escape(row["balance"].as_str().unwrap_or_default())
            )
        })
        .collect();
    let title = escape(info.title.as_deref().unwrap_or("Ledger"));
    Ok(Response::html(format!(
        "<!doctype html><title>{title}</title><h1>{title}</h1><p>Balances on {today}, in {currency}</p><table>{rows}</table>",
        currency = escape(&info.operating_currency),
    )))
}

/// `text` safe to put in HTML: a router's page runs with the user's zhang session
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod test {
    use zhang_plugin_sdk::router::Request;
    use zhang_plugin_sdk::serde_json::json;

    use super::{escape, inventory_text, route};

    #[test]
    fn should_format_inventories_and_escape_html() {
        let cell = json!({"positions": [{"units": {"number": "-10", "currency": "CNY"}, "cost": null}, {"units": {"number": "2", "currency": "AAPL"}}]});
        assert_eq!(inventory_text(&cell), "-10 CNY, 2 AAPL");
        assert_eq!(inventory_text(&json!(null)), "");
        assert_eq!(escape("<a href=\"x\">&</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    }

    #[test]
    fn should_answer_without_the_ledger_where_it_can() {
        let post = Request {
            method: "POST".to_owned(),
            path: "/".to_owned(),
            ..Request::default()
        };
        assert_eq!(route(post).unwrap().status, 405);
        let missing = Request {
            method: "GET".to_owned(),
            path: "/nope".to_owned(),
            ..Request::default()
        };
        assert_eq!(route(missing).unwrap().status, 404);
        // natively the host functions are unavailable
        let page = Request {
            method: "GET".to_owned(),
            path: "/".to_owned(),
            ..Request::default()
        };
        assert!(route(page).is_err());
    }
}
