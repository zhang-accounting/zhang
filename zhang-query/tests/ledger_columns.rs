//! The columns zhang adds to `postings`, `#transactions`, `#entries`, `#balances` and `#documents`
//! for the app's read endpoints (`time`, `timestamp`, `seq`, `id`, `posting_index`,
//! `account_balance`, `balanced`, `errors`, `automatic`), the public valuation API, and the
//! per-ledger cache behind every query. These are
//! zhang extensions, so the expected values are worked out by hand in the comments.

mod common;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use common::{fava_demo_ledger, load_text};
use zhang_core::ast::Directive;
use zhang_core::domains::schemas::{MetaDomain, PriceDomain};
use zhang_core::ledger::Ledger;
use zhang_core::store::{DocumentDomain, DocumentType, TransactionDomain};
use zhang_query::{DataType, ExecuteOptions, Inventory, ParamTypes, Params, PriceMap, Query, QueryErrorKind, Value};

/// A cafe lunch at 10:30 in Shanghai, an unbalanced transaction, one posting to an account
/// that was never opened, two lots bought and partly sold, a pad and a failing balance check.
const LEDGER: &str = r#"
option "operating_currency" "USD"
option "timezone" "Asia/Shanghai"

1970-01-01 commodity USD
1970-01-01 commodity AAPL
1970-01-01 open Assets:Bank
1970-01-01 open Assets:Broker
1970-01-01 open Expenses:Food
1970-01-01 open Equity:Opening

2024-01-05 10:30:00 * "Cafe" "lunch"
  Expenses:Food    12.50 USD
  Assets:Bank

2024-01-06 * "Broken" "does not balance"
  Expenses:Food    10.00 USD
  Assets:Bank     -9.00 USD

2024-01-07 * "Ghost" "unknown account"
  Expenses:Ghost   5.00 USD
  Assets:Bank     -5.00 USD

2024-01-08 * "Broker" "buy"
  Assets:Broker    5 AAPL {100 USD}
  Assets:Broker    5 AAPL {110 USD}
  Assets:Bank   -1050 USD

2024-01-09 * "Broker" "sell"
  Assets:Broker   -7 AAPL {}
  Assets:Bank      740 USD

2024-01-10 balance Assets:Bank 100 USD with pad Equity:Opening
2024-01-11 balance Assets:Bank 1 USD
"#;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
}

fn render(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_owned(),
        Value::Set(set) => format!("{{{}}}", set.iter().cloned().collect::<Vec<_>>().join(", ")),
        other => other.to_string(),
    }
}

fn run(ledger: &Ledger, sql: &str) -> Vec<Vec<Value>> {
    let query = Query::compile(sql).unwrap_or_else(|err| panic!("{sql}: {err}"));
    query
        .execute_at(ledger, &Params::new(), today())
        .unwrap_or_else(|err| panic!("{sql}: {err}"))
        .rows
}

/// The rows as text, one row per line with the cells separated by ` | `.
fn table(ledger: &Ledger, sql: &str) -> String {
    run(ledger, sql)
        .iter()
        .map(|row| row.iter().map(render).collect::<Vec<_>>().join(" | "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn expect(ledger: &Ledger, sql: &str, expected: &str) {
    let expected = expected.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
    assert_eq!(table(ledger, sql), expected, "{sql}");
}

/// Times are in the ledger's timezone, as stored: 10:30 in Shanghai (UTC+8) is 02:30 UTC, Unix
/// time 1704421800; a transaction without a time is at midnight, 16:00 UTC the day before.
#[test]
fn times_are_in_the_ledger_timezone() {
    let ledger = load_text(LEDGER);
    expect(
        &ledger,
        "SELECT DISTINCT date, time, timestamp, narration WHERE date <= 2024-01-06",
        "2024-01-05 | 10:30:00 | 1704421800 | lunch
         2024-01-06 | 00:00:00 | 1704470400 | does not balance",
    );
    // the three tables agree
    let postings = table(&ledger, "SELECT DISTINCT id, time, timestamp");
    let transactions = table(&ledger, "SELECT id, time, timestamp FROM #transactions");
    let entries = table(&ledger, "SELECT id, time, timestamp FROM #entries WHERE type = 'transaction'");
    assert_eq!(postings, transactions);
    assert_eq!(transactions, entries);
    // other directives have their time too: the opens of 1970-01-01 at midnight in Shanghai
    expect(
        &ledger,
        "SELECT type, time, timestamp FROM #entries WHERE type = 'open' LIMIT 1",
        "open | 00:00:00 | -28800",
    );
}

/// A local time that a daylight saving change skips is read with the offset before the
/// change, as zhang stores it: 02:30 on 2024-03-10 in New York is 07:30 UTC, 03:30 EDT.
#[test]
fn times_skipped_by_daylight_saving_are_stored_after_the_gap() {
    let ledger = load_text(
        r#"
option "timezone" "America/New_York"
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
2024-03-10 02:30:00 * "Cafe" "in the gap"
  Expenses:Food    1 USD
  Assets:Bank
"#,
    );
    // 2024-03-10T07:30:00Z
    let expected = "03:30:00 | 1710055800";
    assert_eq!(table(&ledger, "SELECT DISTINCT time, timestamp"), expected);
    assert_eq!(table(&ledger, "SELECT time, timestamp FROM #transactions"), expected);
    assert_eq!(table(&ledger, "SELECT time, timestamp FROM #entries WHERE type = 'transaction'"), expected);
}

/// Daylight saving started at midnight in São Paulo on 2018-11-04, so a directive of that day written
/// without a time is stored at the first time after the gap, 01:00, in every table.
#[test]
fn midnight_skipped_by_daylight_saving_is_the_first_time_after_the_gap() {
    let ledger = load_text(
        r#"
option "timezone" "America/Sao_Paulo"
1970-01-01 open Assets:Bank
1970-01-01 open Equity:Opening
2018-11-04 * "Self" "opening"
  Assets:Bank 1 BRL
  Equity:Opening
2018-11-04 balance Assets:Bank 1 BRL
2018-11-04 document Assets:Bank "statement.pdf"
"#,
    );
    // 2018-11-04T03:00:00Z
    let expected = "01:00:00 | 1541300400";
    assert_eq!(table(&ledger, "SELECT DISTINCT time, timestamp WHERE year = 2018"), expected);
    assert_eq!(table(&ledger, "SELECT DISTINCT time, timestamp FROM #entries WHERE year = 2018"), expected);
    assert_eq!(table(&ledger, "SELECT time, timestamp FROM #balances"), expected);
    assert_eq!(table(&ledger, "SELECT time, timestamp FROM #documents"), expected);
}

/// `seq` is the position of an entry in the order zhang processes the ledger: the four opens and
/// two commodities of 1970 are 0 to 5 (opens first in their day, as beancount orders a day), the
/// transactions 6 to 10, then the padding transaction of the pad 11 and the pad's assertion 12,
/// which zhang checks after its padding, and the failing check 13. `#entries` lists its rows in
/// that order. The correcting transaction zhang stored for a balance check in the past is no
/// entry.
#[test]
fn seq_is_the_position_in_the_processing_order() {
    let ledger = load_text(LEDGER);
    expect(
        &ledger,
        "SELECT seq, type, date FROM #entries WHERE seq >= 4 ORDER BY seq",
        "4 | commodity | 1970-01-01
         5 | commodity | 1970-01-01
         6 | transaction | 2024-01-05
         7 | transaction | 2024-01-06
         8 | transaction | 2024-01-07
         9 | transaction | 2024-01-08
         10 | transaction | 2024-01-09
         11 | transaction | 2024-01-10
         12 | balance | 2024-01-10
         13 | balance | 2024-01-11",
    );
    // the rows of the table come in that order: the assertion after the padding it is checked after
    expect(
        &ledger,
        "SELECT seq, type FROM #entries WHERE date = 2024-01-10",
        "11 | transaction
         12 | balance",
    );
    expect(
        &ledger,
        "SELECT seq, flag, narration FROM #transactions ORDER BY seq DESC LIMIT 2",
        "11 | P | pad Assets:Bank to Equity:Opening
         10 | * | sell",
    );
    // every posting of a transaction shares its seq
    expect(
        &ledger,
        "SELECT seq, count(*) GROUP BY seq ORDER BY seq DESC",
        "11 | 2
         10 | 3
         9 | 3
         8 | 2
         7 | 2
         6 | 2",
    );
    let entries = table(&ledger, "SELECT id, seq FROM #entries WHERE type = 'transaction'");
    assert_eq!(table(&ledger, "SELECT id, seq FROM #transactions"), entries);
    assert_eq!(table(&ledger, "SELECT DISTINCT id, seq"), entries);
    // the period modifiers' own transactions are no entries
    expect(&ledger, "SELECT DISTINCT flag, seq FROM OPEN ON 2024-01-08 WHERE flag = 'S'", "S | NULL");
}

/// `automatic` is whether a posting was written without an amount: the bank leg of the lunch and
/// the padding account's leg of the padding transaction, which takes the -436.50 that brings the
/// bank from -336.50 (-12.50 - 9 - 5 - 1050 + 740) to 100; never a posting of the period modifiers.
#[test]
fn automatic_postings_are_those_written_without_an_amount() {
    let ledger = load_text(LEDGER);
    expect(
        &ledger,
        "SELECT narration, posting_index, account, number, automatic WHERE automatic",
        "lunch | 1 | Assets:Bank | -12.50 | TRUE
         pad Assets:Bank to Equity:Opening | 1 | Equity:Opening | -436.50 | TRUE",
    );
    // every row of a written posting that booking splits across lots is as written
    expect(
        &ledger,
        "SELECT posting_index, number, automatic WHERE narration = 'sell'",
        "0 | -5 | FALSE
         0 | -2 | FALSE
         1 | 740 | FALSE",
    );
    expect(&ledger, "SELECT count(*) FROM OPEN ON 2024-01-08 WHERE flag = 'S' AND automatic", "0");
}

/// A balance assertion of `#balances` has the `id`, `seq`, `time` and `timestamp` of its
/// `#entries` row, so the two tables join on them. The `balance ... with pad` is checked after its
/// padding (seq 11), so it is 12.
#[test]
fn balance_assertions_have_the_id_and_seq_of_their_entry() {
    let ledger = load_text(LEDGER);
    expect(
        &ledger,
        "SELECT seq, account, time, timestamp, passed FROM #balances ORDER BY seq DESC",
        "13 | Assets:Bank | 00:00:00 | 1704902400 | FALSE
         12 | Assets:Bank | 00:00:00 | 1704816000 | TRUE",
    );
    assert_eq!(
        table(&ledger, "SELECT id, seq FROM #balances"),
        table(&ledger, "SELECT id, seq FROM #entries WHERE type = 'balance'")
    );
    let fava = fava_demo_ledger();
    assert_eq!(
        table(&fava, "SELECT id, seq, time FROM #balances"),
        table(&fava, "SELECT id, seq, time FROM #entries WHERE type = 'balance'")
    );
}

/// A document of `#documents` has the `seq`, `time` and `timestamp` of the `document` directive or
/// of the transaction that names it: a transaction's documents share them.
#[test]
fn documents_have_the_seq_and_time_of_what_declares_them() {
    let ledger = load_text(
        r#"
option "timezone" "Asia/Shanghai"
1970-01-01 open Assets:Bank
1970-01-01 open Expenses:Food
2024-01-01 document Assets:Bank "statement.pdf"
2024-01-02 09:15:00 * "Shop" "receipts"
  document: "a.pdf"
  Expenses:Food 1 USD
    document: "b.pdf"
  Assets:Bank
"#,
    );
    expect(
        &ledger,
        "SELECT seq, time, timestamp, source, path FROM #documents ORDER BY seq DESC",
        "3 | 09:15:00 | 1704158100 | transaction | a.pdf
         3 | 09:15:00 | 1704158100 | posting | b.pdf
         2 | 00:00:00 | 1704038400 | directive | statement.pdf",
    );
    assert_eq!(
        table(&ledger, "SELECT DISTINCT seq, time FROM #documents WHERE transaction_id IS NOT NULL"),
        table(&ledger, "SELECT seq, time FROM #transactions")
    );
}

/// The id of a transaction in `#transactions` is the `id` of its postings and of its entry.
#[test]
fn transactions_have_the_id_of_their_postings() {
    let ledger = load_text(LEDGER);
    let ids = run(&ledger, "SELECT id FROM #transactions");
    assert_eq!(ids.len(), 6);
    assert_eq!(ids, run(&ledger, "SELECT DISTINCT id"));
    assert_eq!(ids, run(&ledger, "SELECT id FROM #entries WHERE type = 'transaction'"));
    let fava = fava_demo_ledger();
    assert_eq!(
        run(&fava, "SELECT id FROM #transactions"),
        run(&fava, "SELECT id FROM #entries WHERE type = 'transaction'")
    );
}

/// The sale of 7 AAPL takes the 5 bought at 100 first, then 2 of those at 110: two rows of the
/// written posting 0. `account_balance` is the balance of the posting's account after it,
/// over every posting of that account whatever WHERE and LIMIT keep, while `balance` adds up
/// the rows the query keeps.
#[test]
fn posting_index_and_account_balance() {
    let ledger = load_text(LEDGER);
    expect(
        &ledger,
        "SELECT posting_index, position, account_balance WHERE account = 'Assets:Broker'",
        "0 | 5 AAPL {100 USD, 2024-01-08} | 5 AAPL {100 USD, 2024-01-08}
         1 | 5 AAPL {110 USD, 2024-01-08} | 5 AAPL {100 USD, 2024-01-08}, 5 AAPL {110 USD, 2024-01-08}
         0 | -5 AAPL {100 USD, 2024-01-08} | 5 AAPL {110 USD, 2024-01-08}
         0 | -2 AAPL {110 USD, 2024-01-08} | 3 AAPL {110 USD, 2024-01-08}",
    );
    // the bank: -12.50 - 9.00 - 5.00 - 1050 + 740 + 436.50 (the pad to 100)
    expect(
        &ledger,
        "SELECT date, position, account_balance, balance WHERE account = 'Assets:Bank' AND number > 0",
        "2024-01-09 | 740 USD | -336.50 USD | 740 USD
         2024-01-10 | 436.50 USD | 100.00 USD | 1176.50 USD",
    );
    // unlike balance, it can be read in WHERE: the bank is below zero after five of its postings
    expect(
        &ledger,
        "SELECT count(*) WHERE account = 'Assets:Bank' AND number(only('USD', account_balance)) < 0",
        "5",
    );
    expect(
        &ledger,
        "SELECT account, account_balance ORDER BY seq DESC, posting_index DESC LIMIT 3",
        "Equity:Opening | -436.50 USD
         Assets:Bank | 100.00 USD
         Assets:Bank | -336.50 USD",
    );
    // it is per account, in every currency of the account
    expect(
        &ledger,
        "SELECT account, last(account_balance) GROUP BY account ORDER BY account",
        "Assets:Bank | 100.00 USD
         Assets:Broker | 3 AAPL {110 USD, 2024-01-08}
         Equity:Opening | -436.50 USD
         Expenses:Food | 22.50 USD
         Expenses:Ghost | 5.00 USD",
    );
    // with OPEN ON, over the rows the period produces: the summarized opening balance first
    expect(
        &ledger,
        "SELECT date, flag, account_balance FROM OPEN ON 2024-01-09 WHERE account = 'Assets:Bank'",
        "2024-01-08 | S | -1076.50 USD
         2024-01-09 | * | -336.50 USD
         2024-01-10 | P | 100.00 USD",
    );
}

/// `balanced` is FALSE for the transaction zhang found unbalanced (9.00 against 10.00), and
/// for the sale, whose 740 USD do not match the 5 × 100 + 2 × 110 = 720 USD of cost; `errors`
/// names the kinds zhang recorded for each transaction, as `#errors` does.
#[test]
fn balanced_and_errors_come_from_the_ledger_errors() {
    let ledger = load_text(LEDGER);
    let expected = "lunch | TRUE | {}
                    does not balance | FALSE | {UnbalancedTransaction}
                    unknown account | TRUE | {AccountDoesNotExist}
                    buy | TRUE | {}
                    sell | FALSE | {UnbalancedTransaction}
                    pad Assets:Bank to Equity:Opening | TRUE | {}";
    expect(&ledger, "SELECT narration, balanced, errors FROM #transactions", expected);
    expect(&ledger, "SELECT DISTINCT narration, balanced, errors", expected);
    expect(
        &ledger,
        "SELECT DISTINCT narration WHERE 'AccountDoesNotExist' IN errors OR NOT balanced",
        "does not balance
         unknown account
         sell",
    );
    let kinds = run(
        &ledger,
        "SELECT DISTINCT kind FROM #errors WHERE kind != 'AccountBalanceCheckError' ORDER BY kind",
    );
    let named = run(&ledger, "SELECT errors FROM #transactions")
        .into_iter()
        .flat_map(|row| match &row[0] {
            Value::Set(set) => set.clone(),
            other => panic!("{other:?}"),
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(kinds.into_iter().map(|row| render(&row[0])).collect::<BTreeSet<_>>(), named);
}

/// A query scoped to one account by a parameter returns what the unscoped query returns.
#[test]
fn parameters_scope_postings_to_accounts() {
    let ledger = fava_demo_ledger();
    let types = ParamTypes::new().bind("account", DataType::Str).bind("accounts", DataType::Set);
    let params = Params::new().bind("account", "Assets:US:BofA:Checking").bind(
        "accounts",
        Value::Set(["Assets:US:BofA:Checking".to_owned(), "Liabilities:US:Chase:Slate".to_owned()].into()),
    );
    let execute = |sql: &str| {
        let query = Query::compile_with_params(sql, &types).unwrap();
        query.execute_at(&ledger, &params, today()).unwrap().rows
    };
    let journal = execute("SELECT date, position, balance, account_balance WHERE account = :account ORDER BY seq DESC");
    assert_eq!(journal.len(), 275);
    // the newest first, with the balance of the account
    assert_eq!(journal[0][2], journal[0][3]);
    let both = execute("SELECT account, count(*), last(account_balance) WHERE account IN :accounts GROUP BY account ORDER BY account");
    assert_eq!(both.len(), 2);
    assert_eq!(both[0][1], Value::Int(275));
    let regex = execute("SELECT account, count(*), last(account_balance) WHERE account ~ '^(Assets:US:BofA:Checking|Liabilities:US:Chase:Slate)$' GROUP BY account ORDER BY account");
    assert_eq!(both, regex);
}

/// The public valuation API converts as `convert()` does in a query: here every holding of the
/// fava demo ledger at today's prices, in USD.
#[test]
fn the_valuation_api_converts_like_queries() {
    let ledger = fava_demo_ledger();
    let prices = PriceMap::for_ledger(&ledger);
    let rows = run(
        &ledger,
        "SELECT account, sum(position) AS holdings, convert(sum(position), 'USD') AS usd, convert(sum(position), 'USD', 2015-06-01) AS june
         WHERE account ~ '^Assets' GROUP BY account ORDER BY account",
    );
    assert!(rows.len() > 10);
    let mut converted = 0;
    for row in rows {
        let Value::Inventory(holdings) = &row[1] else { panic!("{row:?}") };
        let usd = holdings.convert("USD", &prices, None);
        assert_eq!(Value::Inventory(usd.clone()), row[2], "{}", row[0]);
        assert_eq!(Value::Inventory(holdings.convert("USD", &prices, NaiveDate::from_ymd_opt(2015, 6, 1))), row[3]);
        // position by position too
        let by_position = holdings
            .positions()
            .map(|it| it.convert("USD", &prices, None))
            .fold(Inventory::new(), |mut total, amount| {
                total.add_amount(&amount);
                total
            });
        assert_eq!(by_position, usd);
        converted += usize::from(holdings.positions().any(|it| it.units.commodity != "USD") && usd.positions().all(|it| it.units.commodity == "USD"));
    }
    assert!(converted > 3, "{converted}");
}

/// The cache of a loaded ledger lives as long as the ledger: a reload starts a new one, so a
/// query sees the changed file.
#[test]
fn a_reload_replaces_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.zhang");
    std::fs::write(&file, LEDGER).unwrap();
    let mut ledger = common::load_ledger(dir.path().to_path_buf(), "main.zhang");
    let count = |ledger: &Ledger| table(ledger, "SELECT count(*), max(seq) FROM #transactions");
    let postings = |ledger: &Ledger| table(ledger, "SELECT count(*), last(account_balance) WHERE account = 'Expenses:Food'");
    assert_eq!(count(&ledger), "6 | 11");
    assert_eq!(postings(&ledger), "2 | 22.50 USD");
    assert!(ledger.derived.is_initialized());

    std::fs::write(
        &file,
        format!("{LEDGER}\n2024-02-01 * \"Cafe\" \"dinner\"\n  Expenses:Food 20.00 USD\n  Assets:Bank\n"),
    )
    .unwrap();
    ledger.reload().unwrap();
    assert!(!ledger.derived.is_initialized());
    assert_eq!(count(&ledger), "7 | 14");
    assert_eq!(postings(&ledger), "3 | 42.50 USD");
}

/// zhang never changes a loaded ledger; code that changes its store after querying it gets an
/// error instead of the rows of the ledger as it was.
#[test]
fn a_ledger_changed_after_its_first_query_is_an_error() {
    let ledger = load_text(LEDGER);
    assert_eq!(table(&ledger, "SELECT count(*)"), "14");
    ledger.store.write().unwrap().prices.clear();
    let mut store = ledger.store.write().unwrap();
    let payee = |txn: &TransactionDomain| match &ledger.directives[txn.directive].data {
        Directive::Transaction(it) => it.payee.as_ref().map(|it| it.as_str().to_owned()),
        _ => None,
    };
    store.transactions.retain(|_, txn| payee(txn).as_deref() != Some("Cafe"));
    drop(store);
    let error = Query::compile("SELECT count(*)")
        .unwrap()
        .execute_at(&ledger, &Params::new(), today())
        .unwrap_err();
    assert!(error.message.contains("the ledger changed"), "{}", error.message);
}

/// Every count the cache's fingerprint keeps catches a change on its own, with the number of
/// transactions unchanged: a price, a document, an error, a metadata entry or a directive more.
/// (The fingerprint counts these, so a change that keeps every count, such as an edited posting
/// amount, is not caught; zhang never changes a loaded ledger, it replaces it.)
#[test]
fn a_ledger_changed_without_changing_its_transactions_is_an_error() {
    /// what is added, and how
    type Change = (&'static str, fn(&mut Ledger));
    let changes: [Change; 5] = [
        ("a price", |ledger| {
            ledger.store.write().unwrap().prices.push(PriceDomain {
                datetime: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
                commodity: "AAPL".to_owned(),
                amount: 120.into(),
                target_commodity: "USD".to_owned(),
            })
        }),
        ("a document", |ledger| {
            let mut store = ledger.store.write().unwrap();
            let txn = store.transactions.values().next().unwrap();
            let Directive::Transaction(booked) = &ledger.directives[txn.directive].data else {
                unreachable!("a stored transaction is a transaction directive")
            };
            let document = DocumentDomain {
                datetime: txn.datetime,
                document_type: DocumentType::Account(booked.postings[0].account.clone()),
                filename: Some("receipt.pdf".to_owned()),
                path: "receipts/receipt.pdf".to_owned(),
                alternate: None,
            };
            store.documents.push(document);
        }),
        ("an error", |ledger| {
            let mut store = ledger.store.write().unwrap();
            let error = store.errors[0].clone();
            store.errors.push(error);
        }),
        ("a metadata entry", |ledger| {
            ledger.store.write().unwrap().metas.push(MetaDomain {
                meta_type: "AccountMeta".to_owned(),
                type_identifier: "Assets:Bank".to_owned(),
                key: "note".to_owned(),
                value: "added".to_owned(),
            })
        }),
        ("a directive", |ledger| {
            let directive = ledger.directives[0].clone();
            ledger.directives.push(directive);
        }),
    ];
    for (change, apply) in changes {
        let mut ledger = load_text(LEDGER);
        assert_eq!(table(&ledger, "SELECT count(*)"), "14", "{change}");
        let transactions = ledger.store.read().unwrap().transactions.len();
        apply(&mut ledger);
        assert_eq!(ledger.store.read().unwrap().transactions.len(), transactions, "{change}");
        match Query::compile("SELECT count(*)").unwrap().execute_at(&ledger, &Params::new(), today()) {
            Ok(_) => panic!("{change} more was not caught"),
            Err(error) => assert!(error.message.contains("the ledger changed"), "{change}: {}", error.message),
        }
    }
}

/// One account holding `lots` lots at distinct costs, one bought per day, paid from a cash
/// account: every posting to the broker adds a lot to its balance.
fn many_lots(lots: usize) -> String {
    let mut ledger = String::from("1970-01-01 commodity USD\n1970-01-01 commodity STK\n1970-01-01 open Assets:Broker\n1970-01-01 open Assets:Cash\n");
    let start = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    for lot in 1..=lots {
        let date = start + chrono::Duration::days(lot as i64);
        ledger.push_str(&format!(
            "\n{date} * \"buy {lot}\"\n  Assets:Broker 1 STK {{{lot}.01 USD}}\n  Assets:Cash -{lot}.01 USD\n"
        ));
    }
    ledger
}

const LOTS: usize = 3000;

fn lots_of(value: &Value) -> usize {
    match value {
        Value::Inventory(inventory) => inventory.len(),
        other => panic!("not an inventory: {other:?}"),
    }
}

/// `account_balance` only builds the balances of the rows a query keeps: with LIMIT, ORDER BY
/// ... LIMIT, or a first()/last() per group, a balance of thousands of lots is copied once per
/// kept row, not once per posting.
#[test]
fn projected_account_balances_are_built_for_the_kept_rows_only() {
    let ledger = load_text(&many_lots(LOTS));
    for (sql, lots) in [
        ("SELECT date, account_balance LIMIT 1", 1),
        ("SELECT date, account_balance ORDER BY date DESC LIMIT 1", LOTS),
        ("SELECT date, account_balance WHERE account = 'Assets:Broker' ORDER BY seq DESC LIMIT 1", LOTS),
        (
            "SELECT date, units(account_balance), account_balance ORDER BY seq DESC LIMIT 1 OFFSET 2",
            LOTS - 1,
        ),
        ("SELECT account, last(account_balance) GROUP BY account ORDER BY account", LOTS),
    ] {
        let query = Query::compile(sql).unwrap();
        let plan = query.explain();
        assert!(plan.contains("account_balance: deferred"), "{sql}\n{plan}");
        let start = Instant::now();
        let rows = query
            .execute_at(&ledger, &Params::new(), today())
            .unwrap_or_else(|err| panic!("{sql}: {err}"))
            .rows;
        // before the balances were deferred, each of these took seconds and gigabytes
        assert!(start.elapsed() < Duration::from_secs(5), "{sql}: {:?}", start.elapsed());
        assert_eq!(lots_of(rows[0].last().unwrap()), lots, "{sql}");
    }
}

/// A query that holds a balance of many lots for many rows stops with "too large" as soon as
/// the balances it holds exceed the result size limit, whether they are deferred or read
/// while scanning (ORDER BY, DISTINCT), instead of copying one balance per posting first.
#[test]
fn account_balances_of_many_lots_fail_fast_when_too_large() {
    let ledger = load_text(&many_lots(LOTS));
    let options = ExecuteOptions {
        today: Some(today()),
        timeout: Some(Duration::from_secs(60)),
        // the rows up to about the 600th already hold more values than this
        max_result_values: Some(200_000),
        count_total: false,
    };
    for sql in [
        "SELECT date, account_balance",
        "SELECT date, account_balance WHERE account = 'Assets:Broker' ORDER BY seq DESC",
        "SELECT date, account_balance ORDER BY account_balance",
        "SELECT DISTINCT account_balance",
    ] {
        let start = Instant::now();
        let error = Query::compile(sql)
            .unwrap()
            .execute_with_options(&ledger, &Params::new(), &options)
            .map(|result| result.rows.len())
            .unwrap_err();
        assert_eq!(error.kind, QueryErrorKind::TooLarge, "{sql}: {}", error.message);
        assert!(start.elapsed() < Duration::from_secs(20), "{sql}: {:?}", start.elapsed());
    }
    // a filter reads the balance of every posting without holding them
    let count = Query::compile("SELECT count(*) WHERE number(only('STK', units(account_balance))) > 2990")
        .unwrap()
        .execute_with_options(&ledger, &Params::new(), &options)
        .unwrap();
    assert_eq!(count.rows, vec![vec![Value::Int(10)]]);
}
