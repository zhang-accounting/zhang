//! The rows of the `postings` table are the booked legs of the directive each stored transaction
//! was processed from (#458), matched to it by position. A stage (a plugin) may put several
//! transactions at one position, or change the legs booking produced; the rows must still be
//! those of the transaction zhang stored, with its prices spread as written.

mod common;

use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Directive, PostingCost, Spanned, Transaction, WrittenPosting, ZhangString};
use zhang_core::ledger::Ledger;
use zhang_query::{Params, Query, Value};

/// Two lots of AAPL in a FIFO account, 10 at 100 USD and 10 at 120 USD.
const LOTS: &str = r#"
1970-01-01 commodity USD
1970-01-01 commodity AAPL

1970-01-01 open Assets:Bank
1970-01-01 open Income:Gains
1970-01-01 open Assets:Fifo
  booking_method: "FIFO"

2024-01-01 * "Broker" "buy"
  Assets:Fifo     10 AAPL {100 USD}
  Assets:Bank  -1000 USD

2024-02-01 * "Broker" "buy again"
  Assets:Fifo     10 AAPL {120 USD}
  Assets:Bank  -1200 USD
"#;

/// A sale of 15 AAPL across both lots at 150 USD per unit: `-10 {100}` and `-5 {120}`, 2250 USD
/// to the bank, a gain of 650 USD.
const SALE_PER_UNIT: &str = r#"
2024-06-01 * "Broker" "sell"
  Assets:Fifo    -15 AAPL {} @ 150 USD
  Assets:Bank   2250 USD
  Income:Gains
"#;

fn run(ledger: &Ledger, sql: &str) -> Vec<Vec<String>> {
    let today = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
    let result = Query::compile(sql)
        .and_then(|query| query.execute_at(ledger, &Params::new(), today))
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
    result.rows.iter().map(|row| row.iter().map(Value::to_string).collect()).collect()
}

fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter().map(|row| row.iter().map(|it| (*it).to_owned()).collect()).collect()
}

/// The rows of the sale: posting index, account, units, lot and per-unit price.
fn sale_rows(ledger: &Ledger) -> Vec<Vec<String>> {
    run(
        ledger,
        "SELECT posting_index, account, number, cost_number, cost_date, price WHERE narration = 'sell'",
    )
}

/// The transaction with narration `narration` among `directives`, by index.
fn transaction_at(directives: &[Spanned<Directive>], narration: &str) -> usize {
    directives
        .iter()
        .position(|directive| matches!(&directive.data, Directive::Transaction(txn) if txn.narration.as_ref().is_some_and(|it| it.as_str() == narration)))
        .unwrap_or_else(|| panic!("no transaction {narration:?}"))
}

/// The rows of the real sale, as the ledger books it: two legs sharing the index of the posting
/// they were written as, then the bank and the interpolated gain.
fn the_sale_booked() -> Vec<Vec<String>> {
    rows(&[
        &["0", "Assets:Fifo", "-10", "100", "2024-01-01", "150 USD"],
        &["0", "Assets:Fifo", "-5", "120", "2024-02-01", "150 USD"],
        &["1", "Assets:Bank", "2250", "NULL", "NULL", "NULL"],
        &["2", "Income:Gains", "-650", "NULL", "NULL", "NULL"],
    ])
}

/// A stage duplicates the sale before it, at the same position, onto an account that holds no
/// lots: the copy cannot be booked (its cost cannot be resolved), so the ledger
/// reports it and never stores it, but it stays among the directives. The stored sale must be
/// read from its own directive, not from the copy, whose legs have no units to make rows of.
#[test]
fn an_unbookable_copy_at_the_same_position_is_passed_over() {
    let ledger = common::load_transformed(&format!("{LOTS}{SALE_PER_UNIT}"), |mut directives| {
        let at = transaction_at(&directives, "sell");
        let Directive::Transaction(sale) = &directives[at].data else { unreachable!() };
        let mut copy = sale.clone();
        copy.narration = Some(ZhangString::quote("DUP"));
        copy.postings[0].account = Account::from_str("Assets:Other").unwrap();
        let span = directives[at].span.clone();
        directives.insert(at, Spanned::new(Directive::Transaction(copy), span));
        directives
    });

    let (stored, errors) = {
        let errors = ledger.errors.iter().map(|it| it.error_type.to_string()).collect::<Vec<_>>();
        (ledger.transactions().len(), errors)
    };
    assert_eq!(stored, 3, "the copy is not stored");
    assert!(
        errors.iter().any(|it| it == "TransactionCannotInferTradeAmount"),
        "the copy is reported as unbookable: {errors:?}"
    );
    assert_eq!(sale_rows(&ledger), the_sale_booked());
    assert_eq!(run(&ledger, "SELECT narration WHERE narration = 'DUP'"), rows(&[]), "the copy has no rows");
    assert_eq!(
        run(&ledger, "SELECT sum(position) WHERE account = 'Assets:Fifo'"),
        rows(&[&["5 AAPL {120 USD, 2024-02-01}"]])
    );
}

/// The ledger with a stage that duplicates the sale before it, at the same position, onto an
/// account that holds no lots: the copy cannot be booked (its cost cannot be resolved), so the
/// ledger reports it and never stores it, but it stays among the directives.
fn with_an_unbookable_copy_of_the_sale() -> Ledger {
    common::load_transformed(&format!("{LOTS}{SALE_PER_UNIT}"), |mut directives| {
        let at = transaction_at(&directives, "sell");
        let Directive::Transaction(sale) = &directives[at].data else { unreachable!() };
        let mut copy = sale.clone();
        copy.narration = Some(ZhangString::quote("DUP"));
        copy.postings[0].account = Account::from_str("Assets:Other").unwrap();
        let span = directives[at].span.clone();
        directives.insert(at, Spanned::new(Directive::Transaction(copy), span));
        directives
    })
}

/// The copy was never stored, so it is no entry: `#entries`, `#transactions` (which `/api/journals`
/// lists) and `#postings` all show the sale, under the id zhang stored it with, with its narration,
/// its accounts and its `seq`. Matching the stored sale to the first directive of its position and
/// date listed the copy under the sale's id, and left the sale's postings without a `seq`.
#[test]
fn an_unbookable_copy_at_the_same_position_takes_neither_the_id_nor_the_seq_of_the_sale() {
    let ledger = with_an_unbookable_copy_of_the_sale();
    let stored = run(&ledger, "SELECT DISTINCT id, seq FROM #postings WHERE narration = 'sell'");
    assert_eq!(stored.len(), 1, "one stored sale: {stored:?}");
    let (id, seq) = (stored[0][0].clone(), stored[0][1].clone());
    assert_ne!(seq, "NULL", "the sale's postings have its seq");

    let june = "date >= 2024-06-01";
    assert_eq!(
        run(&ledger, &format!("SELECT id, seq, narration, accounts FROM #entries WHERE {june}")),
        vec![vec![
            id.clone(),
            seq.clone(),
            "sell".to_owned(),
            "Assets:Bank, Assets:Fifo, Income:Gains".to_owned()
        ]]
    );
    assert_eq!(
        run(&ledger, &format!("SELECT id, seq, narration FROM #transactions WHERE {june}")),
        vec![vec![id, seq, "sell".to_owned()]]
    );
}

/// A stage emits a second sale after the first, at the same position, and it books too (3 of the
/// 5 AAPL left): the two are stored in that order, and each reads its own directive.
#[test]
fn a_booked_copy_at_the_same_position_has_its_own_rows() {
    let ledger = common::load_transformed(&format!("{LOTS}{SALE_PER_UNIT}"), |mut directives| {
        let at = transaction_at(&directives, "sell");
        let Directive::Transaction(sale) = &directives[at].data else { unreachable!() };
        let mut copy = sale.clone();
        copy.narration = Some(ZhangString::quote("sell again"));
        copy.postings[0].units = Some(Amount::new(BigDecimal::from(-3), "AAPL"));
        copy.postings[1].units = Some(Amount::new(BigDecimal::from(450), "USD"));
        let span = directives[at].span.clone();
        directives.insert(at + 1, Spanned::new(Directive::Transaction(copy), span));
        directives
    });
    assert_eq!(ledger.transactions().len(), 4);
    assert_eq!(sale_rows(&ledger), the_sale_booked());
    // the second sale takes 3 of the 5 AAPL left at 120: a gain of 90
    assert_eq!(
        run(
            &ledger,
            "SELECT posting_index, account, number, cost_number, cost_date, price WHERE narration = 'sell again'"
        ),
        rows(&[
            &["0", "Assets:Fifo", "-3", "120", "2024-02-01", "150 USD"],
            &["1", "Assets:Bank", "450", "NULL", "NULL", "NULL"],
            &["2", "Income:Gains", "-90", "NULL", "NULL", "NULL"],
        ])
    );
    assert_eq!(
        run(&ledger, "SELECT sum(position) WHERE account = 'Assets:Fifo'"),
        rows(&[&["2 AAPL {120 USD, 2024-02-01}"]])
    );
}

/// A total price (`@@`) is spread over the units as written, so every leg of the split shows the
/// same per-unit price.
#[test]
fn a_total_price_is_spread_over_the_written_units_of_a_split() {
    let ledger = common::load_text(&format!(
        "{LOTS}
2024-06-01 * \"Broker\" \"sell\"
  Assets:Fifo    -15 AAPL {{}} @@ 2250 USD
  Assets:Bank   2250 USD
  Income:Gains
"
    ));
    assert_eq!(sale_rows(&ledger), the_sale_booked());
}

/// A stage moved a leg of the split away from the other (here the bank posting sits between
/// them): the legs stand on their own, one row each, but each keeps the posting it was written
/// as, and its total price is still spread over the written 15 AAPL, not over its own units.
#[test]
fn a_total_price_is_spread_over_the_written_units_of_a_split_a_stage_broke_apart() {
    let ledger = common::load_transformed(
        &format!(
            "{LOTS}
2024-06-01 * \"Broker\" \"sell\"
  Assets:Fifo    -10 AAPL {{100 USD, 2024-01-01}} @@ 2250 USD
  Assets:Bank   2250 USD
  Assets:Fifo     -5 AAPL {{120 USD, 2024-02-01}} @@ 2250 USD
  Income:Gains  -650 USD
"
        ),
        |mut directives| {
            // the legs as booking left them, with the posting they were written as
            let at = transaction_at(&directives, "sell");
            let Directive::Transaction(sale) = &mut directives[at].data else {
                unreachable!()
            };
            let written = WrittenPosting {
                index: 0,
                units: Some(Amount::new(BigDecimal::from(-15), "AAPL")),
                cost: Some(PostingCost::default()),
            };
            sale.postings[0].written = Some(written.clone());
            sale.postings[2].written = Some(written);
            directives
        },
    );
    assert_eq!(
        sale_rows(&ledger),
        rows(&[
            &["0", "Assets:Fifo", "-10", "100", "2024-01-01", "150 USD"],
            &["1", "Assets:Bank", "2250", "NULL", "NULL", "NULL"],
            &["2", "Assets:Fifo", "-5", "120", "2024-02-01", "150 USD"],
            &["3", "Income:Gains", "-650", "NULL", "NULL", "NULL"],
        ])
    );
    let legs_kept_their_written_form = ledger.directives.iter().any(|directive| match &directive.data {
        Directive::Transaction(Transaction { postings, .. }) => postings.iter().filter(|it| it.written.is_some()).count() == 2,
        _ => false,
    });
    assert!(legs_kept_their_written_form, "booking left the broken-apart legs as they were");
}
