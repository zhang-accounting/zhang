//! Conformance tests for the beancount parser.
//!
//! Two levels of coverage:
//!
//! 1. `official_bean_example_ledger_parses_fully` — beancount's own generated
//!    example ledger (`bean-example`, ~2000 directives; the same file backs the
//!    `fava-demo-ledger` integration fixture) must parse end to end.
//! 2. `supported_language_constructs_parse` — every construct of the beancount
//!    language surface we currently support must parse.
//!
//! Known-unsupported constructs, tracked for full beancount compatibility. They
//! are deliberately NOT silently accepted, because parsing them without proper
//! AST support would drop financially-significant information:
//!   - balance tolerance:  `2014-01-01 balance Assets:Cash 10 ~ 0.01 USD`
//!   - query directive:    `2014-01-01 query "name" "SELECT account"`
//!   - txn keyword flag:   `2014-01-01 txn "payee" "narration"`
//!   - cost lot label:     `{100 USD, "lot1"}`
//!   - total cost:         `{{100 USD}}`
//!   - pushmeta / popmeta

use std::path::PathBuf;

use beancount::parser::parse;

/// beancount's `bean-example` generated ledger (shared with the fava-demo
/// integration fixture).
const OFFICIAL_EXAMPLE: &str = "../../integration-tests/fava-demo-ledger/main.zhang";

#[test]
fn official_bean_example_ledger_parses_fully() {
    let content = std::fs::read_to_string(OFFICIAL_EXAMPLE).expect("read bean-example ledger");
    let directives = parse(&content, None::<PathBuf>).expect("bean-example ledger must parse");
    assert!(directives.len() > 2000, "expected a large ledger, got {} directives", directives.len());
}

#[test]
fn supported_language_constructs_parse() {
    let cases: &[(&str, &str)] = &[
        ("open", "2014-01-01 open Assets:Cash\n"),
        ("open with commodities", "2014-01-01 open Assets:Cash USD,CAD\n"),
        ("open with booking method", "2014-01-01 open Assets:Cash USD \"FIFO\"\n"),
        ("close", "2014-01-01 close Assets:Cash\n"),
        ("commodity", "2014-01-01 commodity USD\n"),
        ("commodity with metadata", "2014-01-01 commodity USD\n  name: \"US Dollar\"\n"),
        ("balance", "2014-01-01 balance Assets:Cash 10 USD\n"),
        ("pad", "2014-01-01 pad Assets:Cash Equity:Open\n"),
        ("note", "2014-01-01 note Assets:Cash \"hello\"\n"),
        ("document", "2014-01-01 document Assets:Cash \"/tmp/x.pdf\"\n"),
        ("price", "2014-01-01 price USD 1.1 CAD\n"),
        ("event", "2014-01-01 event \"location\" \"Paris\"\n"),
        ("custom", "2014-01-01 custom \"budget\" Assets:Cash \"q\" 20.0 TRUE\n"),
        ("transaction flag *", "2014-01-01 * \"payee\" \"narr\"\n  Assets:Cash 1 USD\n  Equity:X\n"),
        ("transaction flag !", "2014-01-01 ! \"narr\"\n  Assets:Cash 1 USD\n  Equity:X\n"),
        ("posting cost", "2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL {100 USD}\n  Equity:X\n"),
        ("posting cost with date", "2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL {100 USD, 2014-01-01}\n  Equity:X\n"),
        ("posting @ price", "2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL @ 100 USD\n  Equity:X\n"),
        ("posting @@ price", "2014-01-01 * \"x\"\n  Assets:Cash 1 HOOL @@ 100 USD\n  Equity:X\n"),
        ("tags and links", "2014-01-01 * \"x\" #tag1 ^link1\n  Assets:Cash 1 USD\n  Equity:X\n"),
        ("posting metadata", "2014-01-01 * \"x\"\n  Assets:Cash 1 USD\n    answer: 42\n  Equity:X\n"),
        ("pushtag / poptag", "pushtag #trip\n2014-01-01 * \"x\"\n  Assets:Cash 1 USD\n  Equity:X\npoptag #trip\n"),
        ("option", "option \"title\" \"My Ledger\"\n"),
        ("plugin", "plugin \"beancount.plugins.auto\" \"config\"\n"),
        ("include", "include \"other.beancount\"\n"),
    ];

    let failed: Vec<&str> = cases.iter().filter(|(_, snippet)| parse(snippet, None::<PathBuf>).is_err()).map(|(name, _)| *name).collect();

    assert!(failed.is_empty(), "these supported constructs failed to parse: {:?}", failed);
}
