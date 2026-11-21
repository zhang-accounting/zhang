//! `zhang.decimal`, the exact decimals of Lua scripts (RFC #698 §4), against the host's own: every result is compared,
//! text and scale included, with `BigDecimal` written by `zhang_shared::decimal::plain_decimal`, as the host writes a
//! number into a query result, an analyzer's table or the budget JSON.

use std::str::FromStr;

use bigdecimal::num_bigint::Sign;
use bigdecimal::{BigDecimal, Zero};
use serde_json::Value;
use zhang_shared::decimal::plain_decimal;
use zhang_testkit::XorShift;

use super::*;

fn post(body: &Value) -> PluginRequest {
    PluginRequest::new("POST", "/", vec![], vec![], serde_json::to_vec(body).unwrap())
}

/// A string of `len` random digits, or of zeros when `zero`.
fn digits(rng: &mut XorShift, len: usize, zero: bool) -> String {
    (0..len).map(|_| if zero { '0' } else { char::from(b'0' + rng.below(10) as u8) }).collect()
}

/// A decimal as a script may write one: with a sign or without; digits, at times with leading zeros; a point with
/// fractional digits, at times with trailing zeros, or none; `.5` and `5.`; zero in several scales. Mostly a few
/// digits, as amounts have, at times up to 80, and now and then close to the 1000 a decimal may have.
fn decimal(rng: &mut XorShift) -> String {
    let sign = *rng.pick(&["", "", "", "", "-", "-", "-", "+"]);
    let (int, fraction) = match rng.below(100) {
        0..=59 => (rng.below(7), rng.below(5)),
        60..=84 => (rng.below(31), rng.below(41)),
        85..=97 => {
            let int = rng.below(81);
            (int, rng.below(81 - int))
        }
        _ => (rng.below(496), rng.below(496)),
    };
    let zero = rng.chance(10);
    let mut int = digits(rng, int, zero);
    if rng.chance(10) {
        int.insert_str(0, &"0".repeat(1 + rng.below(3)));
    }
    let mut fraction = digits(rng, fraction, zero);
    if !fraction.is_empty() && rng.chance(20) {
        fraction.push_str(&"0".repeat(1 + rng.below(5)));
    }
    match (int.is_empty(), fraction.is_empty()) {
        (true, true) => format!("{sign}0"),
        (true, false) if rng.chance(30) => format!("{sign}.{fraction}"),
        (true, false) => format!("{sign}0.{fraction}"),
        (false, true) if rng.chance(5) => format!("{sign}{int}."),
        (false, true) => format!("{sign}{int}"),
        (false, false) => format!("{sign}{int}.{fraction}"),
    }
}

/// `a` with the other sign
fn negated(a: &str) -> String {
    match a.strip_prefix('-') {
        Some(rest) => rest.to_owned(),
        None => format!("-{}", a.strip_prefix('+').unwrap_or(a)),
    }
}

/// The second operand of a case with `a`: another decimal, `a` itself, `-a` (so that a + b cancels), `a` at a larger
/// scale (the same value), or zero in some scale.
fn partner(rng: &mut XorShift, a: &str) -> String {
    match rng.below(100) {
        0..=44 => decimal(rng),
        45..=59 => a.to_owned(),
        60..=74 => negated(a),
        75..=84 if a.len() < 990 => {
            let zeros = "0".repeat(1 + rng.below(4));
            match a.contains('.') {
                true => format!("{a}{zeros}"),
                false => format!("{a}.{zeros}"),
            }
        }
        _ => match (rng.pick(&["", "-"]), rng.below(6)) {
            (sign, 0) => format!("{sign}0"),
            (sign, scale) => format!("{sign}0.{}", "0".repeat(scale)),
        },
    }
}

/// Decimals at the edges of the arithmetic: zeros and signs, the forms of the point, and the largest coefficient that
/// adds in a Lua integer next to the smallest that does not.
const EDGES: &[&str] = &[
    "0",
    "-0",
    "+0",
    "0.00",
    "-0.000",
    ".5",
    "-.5",
    "5.",
    "1",
    "-1",
    "0.1",
    "-0.10",
    "999999999999999999",
    "-999999999999999999",
    "1000000000000000000",
    "99999999999999999.9",
    "0.000000000000000001",
    "-0.0000000000000000001",
    "123456789012345678.123456789012345678",
    "500.00",
];

/// What the parity router answers for `a` and `b`, computed with the host's decimals: `a + b` as BigDecimal's `a + b`
/// and `a - b` as its `&a - &b`, which keep the larger scale of the two for every pair (the rule zhang.decimal keeps),
/// written in plain notation.
fn expected(a: &str, b: &str) -> Value {
    let x = BigDecimal::from_str(a).unwrap_or_else(|e| panic!("{a}: {e}"));
    let y = BigDecimal::from_str(b).unwrap_or_else(|e| panic!("{b}: {e}"));
    let sum = x.clone() + y.clone();
    let difference = &x - &y;
    let larger = x.fractional_digit_count().max(y.fractional_digit_count());
    assert_eq!(
        (sum.fractional_digit_count(), difference.fractional_digit_count()),
        (larger, larger),
        "{a} and {b}: the larger scale"
    );
    let (sum, difference) = (plain_decimal(&sum), plain_decimal(&difference));
    let ordering = x.cmp(&y) as i8;
    let sign = match x.sign() {
        Sign::Minus => -1,
        Sign::NoSign => 0,
        Sign::Plus => 1,
    };
    json!([
        plain_decimal(&x),
        sum,
        difference,
        plain_decimal(&-x.clone()),
        plain_decimal(&x.abs()),
        ordering,
        x == y,
        x < y,
        x <= y,
        x.is_zero(),
        sign,
        sum,
        difference,
        sum,
        difference,
        format!("[\"{sum}\"]"),
    ])
}

#[test]
fn zhang_decimal_computes_as_the_host_does_on_random_decimals() {
    let (_dir, ledger) = router_ledger(&["decimal_parity.lua"]);
    let mut rng = XorShift(0x0698_2026_1121);
    let mut cases = EDGES
        .iter()
        .flat_map(|a| EDGES.iter().map(|b| ((*a).to_owned(), (*b).to_owned())))
        .collect::<Vec<_>>();
    for _ in 0..12_000 {
        let a = decimal(&mut rng);
        let b = partner(&mut rng, &a);
        cases.push((a, b));
    }
    let lists = (0..200)
        .map(|_| (0..rng.below(40)).map(|_| decimal(&mut rng)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let mut compared = 0;
    for chunk in cases.chunks(1000) {
        let body = json!({"cases": chunk.iter().map(|(a, b)| [a, b]).collect::<Vec<_>>()});
        let answer = json_body(&call(&ledger, "decimal-parity", &post(&body)).unwrap());
        let results = answer["results"].as_array().unwrap();
        assert_eq!(results.len(), chunk.len());
        for ((a, b), result) in chunk.iter().zip(results) {
            assert_eq!(result, &expected(a, b), "a = {a}, b = {b}");
            compared += 1;
        }
    }
    assert_eq!(compared, EDGES.len() * EDGES.len() + 12_000);
    // a sum of many, from zero, keeps the largest scale of them all
    let answer = json_body(&call(&ledger, "decimal-parity", &post(&json!({"lists": lists}))).unwrap());
    let sums = lists
        .iter()
        .map(|list| plain_decimal(&list.iter().fold(BigDecimal::zero(), |sum, it| sum + BigDecimal::from_str(it).unwrap())))
        .collect::<Vec<_>>();
    assert_eq!(answer["sums"], json!(sums));
}

/// The line of `text` in the fixture `fixture`, as an error names it: `<fixture>:<line>:`
fn at(fixture: &str, text: &str) -> String {
    let source = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugins").join(fixture)).unwrap();
    let line = source
        .lines()
        .position(|line| line.contains(text))
        .unwrap_or_else(|| panic!("{text} is in {fixture}"))
        + 1;
    format!("{fixture}:{line}: ")
}

#[test]
fn a_decimal_value_is_a_parameter_and_a_text_and_refuses_what_is_not_exact() {
    let (_dir, ledger) = router_ledger(&["decimal_api.lua"]);
    let case = |path: &str| json_body(&call(&ledger, "decimal-api", &get(path)).unwrap());
    let error = |path: &str| {
        let answer = case(path);
        answer["error"].as_str().unwrap_or_else(|| panic!("{path} raises: {answer}")).to_owned()
    };

    // a value from a string in plain notation, an integer, or another value: its text is as the host writes it
    assert_eq!(
        case("/values"),
        json!([
            "7.50",
            "0.5",
            "5",
            "0.00",
            "12",
            "-3",
            "-9223372036854775808",
            true,
            "0",
            "3.25",
            true,
            false,
            false,
            "zhang.decimal",
            "table",
            "decimal",
            "12.5"
        ])
    );
    // nil is the NULL parameter it always was
    assert_eq!(case("/null"), json!([{"type": "decimal", "value": null}, {"type": "decimal", "value": null}]));
    // by value; a string or an integer compares on either side of < and <=, while == is false for two types, and for
    // a decimal value and any other table
    assert_eq!(
        case("/compare"),
        json!([true, true, true, true, false, true, false, 0, -1, 0, -1, true, true, true, false, false])
    );
    // its text everywhere: tostring, `..`, json.encode, and a typed parameter of a query
    assert_eq!(
        case("/text"),
        json!([
            "-0.50",
            "x = -0.50",
            "-0.50!",
            "{\"list\":[\"0.30\"],\"x\":\"-0.50\"}",
            {"echo": "SELECT date WHERE number > :min", "params": {"min": "12.50"}}
        ])
    );

    // what is refused, at the script's line
    assert!(
        error("/float").ends_with(&format!(
            "{}decimal must be a string or an integer, got 0.1",
            at("decimal_api.lua", "local x = d(0.1)")
        )),
        "{}",
        error("/float")
    );
    assert!(
        error("/float-operand").ends_with(&format!(
            "{}decimal must be a string or an integer, got 0.5",
            at("decimal_api.lua", "\"/float-operand\"")
        )),
        "{}",
        error("/float-operand")
    );
    assert!(
        error("/exponent").ends_with(&format!(
            "{}decimal must be a number in plain notation, such as \"-12.50\", got \"1e3\"",
            at("decimal_api.lua", "local x = d(\"1e3\")")
        )),
        "{}",
        error("/exponent")
    );
    let refused = case("/text-refused");
    for message in refused.as_array().unwrap() {
        let message = message.as_str().unwrap();
        assert!(
            message.starts_with("decimal must be a number in plain notation, such as \"-12.50\", got "),
            "{message}"
        );
    }
    assert_eq!(refused[0], "decimal must be a number in plain notation, such as \"-12.50\", got \"\"");
    assert_eq!(refused[5], "decimal must be a number in plain notation, such as \"-12.50\", got \" 1\"");
    // at most 1000 digits are read, as zhang_convert reads them; a sum may be longer
    assert_eq!(
        case("/digits"),
        json!([
            1002,
            format!("0.{}", "0".repeat(500)),
            format!("decimal \"{}\"… (1001 bytes) has more than 1000 digits", "1".repeat(40)),
            format!("decimal \"0.{}\"… (1002 bytes) has more than 1000 digits", "0".repeat(38)),
            format!("decimal \"{}\"… (2000 bytes) has more than 1000 digits", "1".repeat(40)),
        ])
    );
    assert!(
        error("/multiply").ends_with(&format!(
            "{}zhang.decimal has no multiplication: compute it in the host, with a query or zhang.convert",
            at("decimal_api.lua", "\"/multiply\"")
        )),
        "{}",
        error("/multiply")
    );
    assert!(error("/divide").contains("zhang.decimal has no division"), "{}", error("/divide"));
    assert!(
        error("/null-arithmetic").ends_with("a NULL decimal has no value to compute with"),
        "{}",
        error("/null-arithmetic")
    );
    assert!(
        error("/changed").ends_with("a decimal value whose text was replaced: make decimals with zhang.decimal"),
        "{}",
        error("/changed")
    );
    assert!(error("/sealed").ends_with("cannot change a protected metatable"), "{}", error("/sealed"));
    // the value of a text is kept, and shared by none: what a script changes in its copy changes nothing else
    assert_eq!(case("/kept"), json!(["7.5", "7.5", "8.5", "0"]));
    // nor does replacing the library functions, or those of its json table, change what zhang.decimal computes
    assert_eq!(
        case("/patched"),
        json!([
            "3.75",
            format!("2{}1.0", "0".repeat(29)),
            format!("-1{}.75", "0".repeat(30)),
            "decimal must be a string or an integer, got 0.5",
            {"type": "decimal", "value": null}
        ])
    );
    assert!(
        error("/table-operand").ends_with("decimal must be a string or an integer, got table"),
        "{}",
        error("/table-operand")
    );
}

/// What zhang.decimal costs per operation in the interpreter, next to what a batched host operation would cost a
/// script instead (a crossing, and the JSON of the amounts both ways) and the host's own arithmetic. Printed, not
/// checked: `cargo test --release -p zhang-core --features plugin_runtime --test core -- --ignored --nocapture the_cost_of`
#[test]
#[ignore = "performance smoke test: cargo test --release -p zhang-core --features plugin_runtime --test core -- --ignored the_cost_of"]
fn the_cost_of_zhang_decimal() {
    let (_dir, ledger) = router_ledger(&["decimal_bench.lua"]);
    let time = |work: &str, n: usize| {
        let request = PluginRequest::new("GET", format!("/{work}"), vec![("n".to_owned(), n.to_string())], vec![], vec![]);
        let mut samples = (0..7)
            .map(|_| {
                let started = Instant::now();
                call(&ledger, "decimal-bench", &request).unwrap();
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort();
        samples[3]
    };
    println!("a call that does nothing: {:?}", time("noop", 0));
    for (work, n) in [
        ("noop", 1_000_000),
        ("add", 100_000),
        ("add_string", 100_000),
        ("running", 100_000),
        ("long", 100_000),
        ("parse", 100_000),
        ("compare", 100_000),
        ("sum", 100_000),
        ("host", 10_000),
        ("json", 100_000),
    ] {
        let (base, busy) = (time(work, 0), time(work, n));
        println!(
            "{work}: {:.0} ns each ({n} in {busy:?})",
            busy.saturating_sub(base).as_nanos() as f64 / n as f64
        );
    }
    let started = Instant::now();
    let mut total = BigDecimal::zero();
    for _ in 0..100_000 {
        total += BigDecimal::from_str("12.34").unwrap();
        std::hint::black_box(plain_decimal(&total));
    }
    println!(
        "the host reading, adding and writing an amount: {:.0} ns each",
        started.elapsed().as_nanos() as f64 / 100_000.0
    );
}
