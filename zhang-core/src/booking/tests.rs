//! Booked postings (booking-split design, #423 §4): the booker rewrites the postings of a
//! transaction into their booked form, and booking a booked stream again changes nothing.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use bigdecimal::{BigDecimal, RoundingMode};
use indoc::indoc;
use itertools::Itertools;
use zhang_ast::{Currency, Directive, Posting, PostingCost, Rounding, Spanned};

use super::{BookOutcome, Booker};
use crate::data_source::LocalFileSystemDataSource;
use crate::data_type::text::ZhangDataType;
use crate::data_type::DataType;
use crate::inventory::BookingMethod;
use crate::ledger::Ledger;
use crate::store::CommodityLotRecord;

/// what booking a stream produced
struct Booked {
    /// the stream, its transactions booked
    directives: Vec<Spanned<Directive>>,
    lots: HashMap<String, Vec<CommodityLotRecord>>,
    /// per transaction in stream order: the kind of an unbookable one, and the errors, as text
    reports: Vec<Vec<String>>,
    /// per transaction in stream order: the residual of a booked one, rounded at the precision of
    /// each commodity (2 for one not defined), as text
    residuals: Vec<Vec<String>>,
}

/// fold `directives` through a fresh booker, as the store fold does
fn book_stream(default_method: BookingMethod, commodities: &[(String, i32, Rounding)], directives: &[Spanned<Directive>]) -> Booked {
    let mut booker = Booker::new(default_method);
    for (name, precision, rounding) in commodities {
        booker.define_commodity(name, *precision, *rounding);
    }
    let mut directives = directives.to_vec();
    let mut reports = vec![];
    let mut residuals = vec![];
    for directive in &mut directives {
        match &mut directive.data {
            Directive::Open(open) => {
                booker.apply_open(open);
            }
            Directive::Transaction(txn) => {
                let outcome = booker.book(txn);
                reports.push(report(&outcome));
                residuals.push(residual(&outcome, commodities));
            }
            _ => {}
        }
    }
    Booked {
        directives,
        lots: booker.into_lots(),
        reports,
        residuals,
    }
}

/// the residual of a booked transaction per commodity, rounded at the commodity's precision as the
/// store fold's balance check rounds it; empty for an unbookable one
fn residual(outcome: &BookOutcome, commodities: &[(String, i32, Rounding)]) -> Vec<String> {
    let BookOutcome::Booked(booked) = outcome else { return vec![] };
    booked
        .residual
        .iter()
        .map(|(commodity, number): (&Currency, &BigDecimal)| {
            let precision = commodities
                .iter()
                .find(|(name, _, _)| name == commodity)
                .map_or(2, |(_, precision, _)| *precision);
            format!(
                "{} {commodity}",
                number.with_scale_round(i64::from(precision), RoundingMode::HalfUp).normalized()
            )
        })
        .collect()
}

fn report(outcome: &BookOutcome) -> Vec<String> {
    let error = |error: &super::BookingError| format!("{:?} {:?}", error.kind, error.metas.iter().collect::<BTreeMap<_, _>>());
    match outcome {
        BookOutcome::Booked(booked) => booked.errors.iter().map(error).collect(),
        BookOutcome::Unbookable { kind, errors } => std::iter::once(format!("unbookable {kind:?}")).chain(errors.iter().map(error)).collect(),
    }
}

/// the postings of every transaction of the stream, as [`show`]s them
pub(crate) fn postings(directives: &[Spanned<Directive>]) -> Vec<Vec<String>> {
    directives
        .iter()
        .filter_map(|it| match &it.data {
            Directive::Transaction(txn) => Some(txn.postings.iter().map(show).collect()),
            _ => None,
        })
        .collect()
}

/// `account units {cost}`, then ` <- #index units {cost}` as written when booking changed the
/// posting; `?` for no units
pub(crate) fn show(posting: &Posting) -> String {
    let units = |units: &Option<zhang_ast::amount::Amount>| units.as_ref().map_or("?".to_owned(), ToString::to_string);
    let cost = |cost: &Option<PostingCost>| cost.as_ref().map(|it| format!(" {}", show_cost(it))).unwrap_or_default();
    let mut out = format!("{} {}{}", posting.account.name(), units(&posting.units), cost(&posting.cost));
    if let Some(written) = &posting.written {
        out.push_str(&format!(" <- #{} {}{}", written.index, units(&written.units), cost(&written.cost)));
    }
    out
}

fn show_cost(cost: &PostingCost) -> String {
    let mut parts = vec![];
    if let Some(base) = &cost.base {
        parts.push(base.to_string());
    }
    if let Some(date) = &cost.date {
        parts.push(date.naive_date().to_string());
    }
    if let Some(label) = &cost.label {
        parts.push(format!("{label:?}"));
    }
    let parts = parts.join(", ");
    if cost.total {
        format!("{{{{{parts}}}}}")
    } else {
        format!("{{{parts}}}")
    }
}

/// lots of one account, as `units {cost, date, "label"}`
fn lots(booked: &Booked, account: &str) -> Vec<String> {
    booked
        .lots
        .get(account)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(super::describe_lot)
        .collect()
}

fn parse(text: &str) -> Vec<Spanned<Directive>> {
    ZhangDataType {}.transform(text.to_owned(), None).expect("the ledger parses")
}

const COMMODITIES: &[(&str, i32)] = &[("USD", 2), ("CNY", 2)];

fn book_text(text: &str) -> Booked {
    let commodities = COMMODITIES
        .iter()
        .map(|(name, precision)| ((*name).to_owned(), *precision, Rounding::RoundDown))
        .collect_vec();
    book_stream(BookingMethod::Fifo, &commodities, &parse(text))
}

/// the prefix of the fixtures the local file system data source cannot load: glob includes need
/// the opendal data source of zhang-cli
const NOT_LOADABLE_HERE: &str = "wildcard-include";

fn load(dir: PathBuf, name: &str) -> Ledger {
    let source = Arc::new(LocalFileSystemDataSource::new(ZhangDataType {}));
    Ledger::load_with_data_source(dir, "main.zhang".to_owned(), source).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// every fixture ledger of the repository, loaded, with its name: the integration tests zhang-core
/// can load on its own, and the example ledger
fn fixture_ledgers() -> Vec<(String, Ledger)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut dirs = std::fs::read_dir(root.join("integration-tests"))
        .unwrap()
        .map(|it| it.unwrap().path())
        .filter(|it| it.join("main.zhang").exists() && !it.file_name().is_some_and(|name| name.to_string_lossy().starts_with(NOT_LOADABLE_HERE)))
        .collect_vec();
    dirs.sort();
    let mut ledgers = dirs
        .into_iter()
        .map(|dir| {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let ledger = load(dir, &name);
            (name, ledger)
        })
        .collect_vec();
    // the example ledger includes a file the repository does not have, which the opendal data
    // source of zhang-cli tolerates and the local one does not: loaded without its includes
    let example = std::fs::read_to_string(root.join("examples/main.zhang")).unwrap();
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::write(dir.join("main.zhang"), example.lines().filter(|line| !line.starts_with("include ")).join("\n")).unwrap();
    ledgers.push(("examples".to_owned(), load(dir, "examples")));
    ledgers
}

/// The table of design §4: what booking writes for each kind of posting.
#[test]
fn booking_rewrites_the_postings_into_their_booked_form() {
    let booked = book_text(indoc! {r#"
        1970-01-01 open Assets:A
        1970-01-01 open Assets:B
        1970-01-01 open Income:I

        2024-05-16 * "buy at a per-unit cost without a date"
          Assets:A 10 USD { 10 CNY }
          Income:I -100 CNY

        2024-05-17 * "buy at a total cost, with the price written on the other posting"
          Assets:A 10 USD {{ 110 CNY }}
          Income:I

        2024-05-18 * "sell across both lots"
          Assets:A -15 USD {}
            note: "both lots"
          Income:I

        2024-05-19 * "sell more than is held at that cost"
          Assets:A -8 USD { 11 CNY }
          Income:I 88 CNY

        2024-05-20 * "hold units without a cost, and a price"
          Assets:B 6 USD
          Assets:B 1 USD @ 7 CNY
          Income:I -13 CNY

        2024-05-21 * "reduce with {} an account holding nothing at cost"
          Assets:B -3 USD {}
          Income:I 3 USD
    "#});

    assert_eq!(
        postings(&booked.directives),
        vec![
            // an augmentation: its cost resolved to the lot's per-unit cost and the transaction date
            vec!["Assets:A 10 USD {10 CNY, 2024-05-16} <- #0 10 USD {10 CNY}", "Income:I -100 CNY"],
            // a total cost becomes the per-unit cost; the implicit posting gets its interpolated units
            vec!["Assets:A 10 USD {11 CNY, 2024-05-17} <- #0 10 USD {{110 CNY}}", "Income:I -110 CNY <- #1 ?"],
            // a reduction over two lots: one leg per lot, adjacent, sharing the written index
            vec![
                "Assets:A -10 USD {10 CNY, 2024-05-16} <- #0 -15 USD {}",
                "Assets:A -5 USD {11 CNY, 2024-05-17} <- #0 -15 USD {}",
                "Income:I 155 CNY <- #1 ?",
            ],
            // the remainder no lot covers: a leg on the lot the booker opens for it, dated by the sale
            vec![
                "Assets:A -5 USD {11 CNY, 2024-05-17} <- #0 -8 USD {11 CNY}",
                "Assets:A -3 USD {11 CNY, 2024-05-19} <- #0 -8 USD {11 CNY}",
                "Income:I 88 CNY",
            ],
            // postings without a cost are left as written
            vec!["Assets:B 6 USD", "Assets:B 1 USD", "Income:I -13 CNY"],
            // a `{}` remainder keeps its `{}` (E9), so the posting is as written
            vec!["Assets:B -3 USD {}", "Income:I 3 USD"],
        ]
    );
    assert_eq!(lots(&booked, "Assets:A"), vec!["-3 USD {11 CNY, 2024-05-19}"]);
    assert_eq!(lots(&booked, "Assets:B"), vec!["7 USD", "-3 USD"]);
    // every leg of a split carries the meta of the posting it was written as: the written form
    // merges the legs back with the first leg's meta, and nothing is lost on the others
    let Directive::Transaction(sale) = &booked.directives[5].data else {
        panic!("the sale")
    };
    for leg in &sale.postings[..2] {
        assert_eq!(leg.meta.get_one("note").map(|it| it.as_str()), Some("both lots"));
    }
    assert_eq!(
        booked.reports,
        vec![
            vec![],
            vec![],
            vec![],
            vec!["NoEnoughCommodityLot {\"transaction_amount\": \"-8\"}".to_owned()],
            vec![],
            vec!["NoEnoughCommodityLot {\"transaction_amount\": \"-3\"}".to_owned()],
        ]
    );

    // booking the booked stream again changes nothing: every leg names the lot it was booked
    // against, and the errors come out the same
    let commodities = COMMODITIES
        .iter()
        .map(|(name, precision)| ((*name).to_owned(), *precision, Rounding::RoundDown))
        .collect_vec();
    assert_same_booking(&book_stream(BookingMethod::Fifo, &commodities, &booked.directives), &booked, "the table ledger");
}

/// `again`, the booking of `once`'s stream, equals `once`: the whole transactions (postings with
/// their written forms, flags, metas), the lots, the errors and the residuals
fn assert_same_booking(again: &Booked, once: &Booked, name: &str) {
    let transactions = |booked: &Booked| booked.directives.iter().map(|it| it.data.clone()).collect_vec();
    assert_eq!(transactions(again), transactions(once), "{name}: the directives");
    assert_eq!(again.lots, once.lots, "{name}: the lots");
    assert_eq!(again.reports, once.reports, "{name}: the errors");
    assert_eq!(again.residuals, once.residuals, "{name}: the residuals");
}

/// A posting a plugin left without units, or with a `{}` cost, in a transaction whose other
/// postings are booked is completed, and the legs of a split posting keep grouping by their index.
#[test]
fn an_unbooked_posting_among_booked_ones_is_completed() {
    let booked = book_text(indoc! {r#"
        1970-01-01 open Assets:A
        1970-01-01 open Income:I

        2024-05-16 * "buy"
          Assets:A 10 USD { 10 CNY }
          Assets:A 10 USD { 11 CNY }
          Income:I -210 CNY

        2024-05-18 * "sell"
          Assets:A -15 USD {}
          Income:I
    "#});
    let mut directives = booked.directives.clone();
    // a stage drops the interpolated units of the income posting again
    let Directive::Transaction(sale) = &mut directives[3].data else {
        panic!("the sale")
    };
    sale.postings[2].units = None;
    sale.postings[2].written = None;

    let commodities = COMMODITIES
        .iter()
        .map(|(name, precision)| ((*name).to_owned(), *precision, Rounding::RoundDown))
        .collect_vec();
    let again = book_stream(BookingMethod::Fifo, &commodities, &directives);
    assert_eq!(
        postings(&again.directives)[1],
        vec![
            "Assets:A -10 USD {10 CNY, 2024-05-16} <- #0 -15 USD {}",
            "Assets:A -5 USD {11 CNY, 2024-05-16} <- #0 -15 USD {}",
            // its position: no leg of the transaction carries that index
            "Income:I 155 CNY <- #2 ?",
        ]
    );
    assert_eq!(again.lots, booked.lots);
}

/// `book(book(s)) == book(s)` over every fixture ledger: the postings, the lots and the errors.
#[test]
fn booking_a_booked_stream_again_changes_nothing_on_every_fixture_ledger() {
    let mut rewritten = 0;
    for (name, ledger) in fixture_ledgers() {
        let commodities = ledger
            .operations()
            .read()
            .commodities
            .values()
            .map(|it| (it.name.clone(), it.precision, it.rounding))
            .collect_vec();
        let once = book_stream(ledger.options.default_booking_method, &commodities, &ledger.directives);
        let twice = book_stream(ledger.options.default_booking_method, &commodities, &once.directives);
        assert_same_booking(&twice, &once, &name);
        rewritten += postings(&once.directives).iter().flatten().filter(|it| it.contains(" <- ")).count();
        // the legs of a split all carry the account, flag, price, comment and meta of the posting
        // they were written as: the written form merges them back with the first leg's, and
        // nothing is lost on the others
        for directive in &once.directives {
            let Directive::Transaction(booked) = &directive.data else { continue };
            for group in super::written_groups(&booked.postings) {
                let legs = group.legs;
                let first = &legs[0];
                for leg in &legs[1..] {
                    assert_eq!(leg.account, first.account, "{name}: the legs of a split stay on one account");
                    assert_eq!(
                        (&leg.flag, &leg.price, &leg.comment, &leg.meta),
                        (&first.flag, &first.price, &first.comment, &first.meta),
                        "{name}: every leg of a split keeps the written posting's flag, price, comment and meta"
                    );
                }
            }
        }
    }
    assert!(rewritten > 20, "the fixtures exercise booking: {rewritten} postings rewritten");
}

fn book_twice(text: &str) -> (Booked, Booked) {
    let commodities = COMMODITIES
        .iter()
        .map(|(name, precision)| ((*name).to_owned(), *precision, Rounding::RoundDown))
        .collect_vec();
    let once = book_text(text);
    let again = book_stream(BookingMethod::Fifo, &commodities, &once.directives);
    (once, again)
}

/// Current behavior, pinned: an implicit posting that carries a cost spec (`Assets:A { 10 CNY }`,
/// which the parser accepts) books the default lot of the weight commodity, its spec ignored
/// (beancount would interpolate its units from the cost). Its leg carries no cost, so booking it
/// again books the same lot; the spec stays in the written form.
#[test]
fn current_behavior_an_implicit_posting_with_a_cost_spec_books_the_default_lot() {
    let (once, again) = book_twice(indoc! {r#"
        1970-01-01 open Assets:A
        1970-01-01 open Income:I

        2024-05-16 * "an implicit posting that carries a cost spec"
          Income:I -100 CNY
          Assets:A { 10 CNY }
    "#});
    assert_eq!(postings(&once.directives), vec![vec!["Income:I -100 CNY", "Assets:A 100 CNY <- #1 ? {10 CNY}"]]);
    assert_eq!(lots(&once, "Assets:A"), vec!["100 CNY"]);
    assert_eq!(once.reports, vec![Vec::<String>::new()]);
    assert_eq!(once.residuals, vec![vec!["0 CNY"]]);
    assert_same_booking(&again, &once, "implicit with a cost spec");
}

/// Lots bought at a total cost carry a long per-unit cost (`{{100 CNY}}` over 3 units). Booking
/// the booked stream again interpolates and balances at the written scale, not at the lot cost's.
#[test]
fn total_cost_lots_rebook_without_division_dust() {
    let (once, again) = book_twice(indoc! {r#"
        1970-01-01 open Assets:A
        1970-01-01 open Assets:Z
        1970-01-01 open Income:I

        2024-05-16 * "non-terminating per-unit"
          Assets:A 3 USD {{ 100 CNY }}
          Income:I

        2024-05-17 * "sell it all with {}"
          Assets:A -3 USD {}
          Income:I

        2024-05-18 * "zero units at a total cost"
          Assets:Z 0 USD {{ 100 CNY }}
          Income:I

        2024-05-20 * "non-terminating per-unit, explicit other side"
          Assets:A 7 USD {{ 100 CNY }}
          Income:I -100 CNY

        2024-05-21 * "sell with explicit CNY"
          Assets:A -7 USD {}
          Income:I 100 CNY
    "#});
    let shown = postings(&once.directives);
    assert_eq!(shown[0][1], "Income:I -100 CNY <- #1 ?");
    assert_eq!(shown[1][1], "Income:I 100 CNY <- #1 ?", "the sale of the lot weighs its total, dust dropped");
    assert_eq!(
        shown[2],
        vec!["Assets:Z 0 USD {100 CNY, 2024-05-18} <- #0 0 USD {{100 CNY}}", "Income:I 0 CNY <- #1 ?"]
    );
    assert_eq!(shown[4][1], "Income:I 100 CNY");
    assert_eq!(lots(&once, "Assets:A"), Vec::<String>::new());
    assert!(once.reports.iter().all(Vec::is_empty), "{:?}", once.reports);
    assert!(once.residuals.iter().all(|it| it == &["0 CNY"]), "{:?}", once.residuals);
    assert_same_booking(&again, &once, "total cost lots");

    // a stage drops the explicit units of a posting next to a long lot cost: completing it
    // rounds at the written scale, not at the lot cost's
    let mut directives = once.directives.clone();
    let Directive::Transaction(sale) = &mut directives[7].data else {
        panic!("the sale with explicit CNY")
    };
    sale.postings[1].units = None;
    sale.postings[1].written = None;
    let commodities = COMMODITIES
        .iter()
        .map(|(name, precision)| ((*name).to_owned(), *precision, Rounding::RoundDown))
        .collect_vec();
    let completed = book_stream(BookingMethod::Fifo, &commodities, &directives);
    assert_eq!(postings(&completed.directives)[4][1], "Income:I 100 CNY <- #1 ?");
}

/// Labelled and unlabelled lots of the same cost and date: a split across them books each leg
/// against its own lot again, under FIFO and LIFO.
#[test]
fn labelled_lot_splits_rebook_to_the_same_lots() {
    let (once, again) = book_twice(indoc! {r#"
        1970-01-01 open Assets:F
        1970-01-01 open Assets:L
          booking_method: "LIFO"
        1970-01-01 open Income:I

        2024-05-16 * "F: unlabelled then labelled, same cost and date"
          Assets:F 10 USD { 10 CNY }
          Assets:F 10 USD { 10 CNY, "a" }
          Income:I -200 CNY

        2024-05-16 * "L: labelled then unlabelled, same cost and date"
          Assets:L 10 USD { 10 CNY, "a" }
          Assets:L 10 USD { 10 CNY }
          Income:I -200 CNY

        2024-05-17 * "F sells across both"
          Assets:F -15 USD {}
          Income:I

        2024-05-17 * "L sells across both"
          Assets:L -15 USD {}
          Income:I

        2024-05-18 * "L sells by label"
          Assets:L -2 USD {, "a"}
          Income:I

        2024-05-19 * "F sells rest, split by lot"
          Assets:F -5 USD {}
          Income:I
    "#});
    let shown = postings(&once.directives);
    assert_eq!(
        shown[2],
        vec![
            "Assets:F -10 USD {10 CNY, 2024-05-16} <- #0 -15 USD {}",
            "Assets:F -5 USD {10 CNY, 2024-05-16, \"a\"} <- #0 -15 USD {}",
            "Income:I 150 CNY <- #1 ?",
        ]
    );
    assert_eq!(
        shown[3],
        vec![
            "Assets:L -10 USD {10 CNY, 2024-05-16} <- #0 -15 USD {}",
            "Assets:L -5 USD {10 CNY, 2024-05-16, \"a\"} <- #0 -15 USD {}",
            "Income:I 150 CNY <- #1 ?",
        ]
    );
    assert_eq!(shown[4][0], "Assets:L -2 USD {10 CNY, 2024-05-16, \"a\"} <- #0 -2 USD {\"a\"}");
    assert_eq!(lots(&once, "Assets:F"), Vec::<String>::new());
    assert_eq!(lots(&once, "Assets:L"), vec!["3 USD {10 CNY, 2024-05-16, \"a\"}"]);
    assert!(once.reports.iter().all(Vec::is_empty), "{:?}", once.reports);
    assert_same_booking(&again, &once, "labelled lots");
}
