//! Parser for zhang's native text format.
//!
//! This is a hand-written recursive-descent parser built on [`nom`]. It replaces
//! the previous `pest` + `pest_consume` grammar (`zhang.pest`) while producing
//! exactly the same [`Directive`] AST — the shared test module below is the
//! behavioural contract.

use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while, take_while1, take_while_m_n};
use nom::character::complete::{char, line_ending, not_line_ending, one_of, satisfy, space0, space1};
use nom::combinator::{map, map_res, opt, recognize, value};
use nom::multi::{many0, many1, many_m_n, separated_list1};
use nom::sequence::{delimited, pair, preceded, terminated, tuple};
use nom::IResult;
use snailquote::unescape;
use zhang_ast::amount::Amount;
use zhang_ast::*;

/// Error returned when the input cannot be parsed as zhang's text format.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// Byte offset of `sub` (which must be a sub-slice of `original`) within `original`.
fn offset(original: &str, sub: &str) -> usize {
    sub.as_ptr() as usize - original.as_ptr() as usize
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

// ---------------------------------------------------------------------------
// low level tokens
// ---------------------------------------------------------------------------

/// A whitespace-only line together with its terminating newline.
fn blank_line(i: &str) -> IResult<&str, ()> {
    value((), pair(space0, line_ending))(i)
}

/// `comment_prefix = ";" | "*" | "#" | "//"`
fn comment_prefix(i: &str) -> IResult<&str, &str> {
    alt((tag("//"), tag(";"), tag("*"), tag("#")))(i)
}

/// An inline comment (prefix + rest of line), the whole of which is discarded.
fn inline_comment(i: &str) -> IResult<&str, ()> {
    value((), pair(comment_prefix, not_line_ending))(i)
}

/// Trailing `space* comment?` allowed after a single-line directive.
fn line_trailer(i: &str) -> IResult<&str, ()> {
    value((), pair(space0, opt(inline_comment)))(i)
}

/// `valuable_comment = space* comment_prefix space* comment_value`, returning the
/// comment body (`comment_value`).
fn valuable_comment(i: &str) -> IResult<&str, String> {
    let (i, _) = space0(i)?;
    valuable_comment_body(i)
}

/// The `comment_prefix space* comment_value` portion, assuming any leading spaces
/// are already consumed.
fn valuable_comment_body(i: &str) -> IResult<&str, String> {
    let (i, _) = comment_prefix(i)?;
    let (i, _) = space0(i)?;
    let (i, body) = not_line_ending(i)?;
    Ok((i, body.to_string()))
}

/// `unquote_string`: a bare word terminated by whitespace, quote, colon, paren or
/// comma.
fn unquote_string_raw(i: &str) -> IResult<&str, &str> {
    take_while1(|c: char| !matches!(c, '"' | ':' | '(' | ')' | ',' | ' ' | '\t' | '\n' | '\r'))(i)
}

/// A single literal character or escape sequence inside a quoted string.
fn string_char(i: &str) -> IResult<&str, &str> {
    alt((
        recognize(pair(
            char('\\'),
            alt((
                recognize(one_of("\"\\/bfnrt")),
                recognize(tuple((char('u'), char('{'), take_while_m_n(4, 4, |c: char| c.is_ascii_hexdigit()), char('}')))),
                recognize(pair(char('u'), take_while_m_n(4, 4, |c: char| c.is_ascii_hexdigit()))),
            )),
        )),
        recognize(satisfy(|c: char| c != '"' && c != '\\')),
    ))(i)
}

/// `quote_string = "\"" inner "\""`, unescaped via `snailquote`.
fn quote_string(i: &str) -> IResult<&str, ZhangString> {
    let (rest, raw) = recognize(tuple((char('"'), many0(string_char), char('"'))))(i)?;
    let unescaped = unescape(raw).expect("string contains invalid escape char");
    Ok((rest, ZhangString::QuoteString(unescaped)))
}

/// `string = unquote_string | quote_string`
fn string(i: &str) -> IResult<&str, ZhangString> {
    alt((map(unquote_string_raw, |s: &str| ZhangString::UnquoteString(s.to_string())), quote_string))(i)
}

/// `commodity_name = ASCII_ALPHA (ASCII_ALPHANUMERIC | "." | "_" | "-" | "'")*`
fn commodity_name(i: &str) -> IResult<&str, String> {
    map(
        recognize(pair(
            satisfy(|c: char| c.is_ascii_alphabetic()),
            take_while(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '\'')),
        )),
        |s: &str| s.to_string(),
    )(i)
}

/// `account_type = "Assets" | "Liabilities" | "Equity" | "Income" | "Expenses"`
fn account_type(i: &str) -> IResult<&str, &str> {
    alt((tag("Assets"), tag("Liabilities"), tag("Equity"), tag("Income"), tag("Expenses")))(i)
}

/// `account_name = account_type (":" unquote_string)+`
fn account_name(i: &str) -> IResult<&str, Account> {
    let (i, account_type) = account_type(i)?;
    let (i, components) = many1(preceded(char(':'), map(unquote_string_raw, |s: &str| s.to_string())))(i)?;
    let content = format!("{}:{}", account_type, components.join(":"));
    Ok((
        i,
        Account {
            account_type: AccountType::from_str(account_type).expect("invalid account type"),
            content,
            components,
        },
    ))
}

// ---------------------------------------------------------------------------
// dates
// ---------------------------------------------------------------------------

fn date_only_raw(i: &str) -> IResult<&str, &str> {
    recognize(tuple((
        take_while_m_n(4, 4, is_digit),
        char('-'),
        take_while_m_n(1, 2, is_digit),
        char('-'),
        take_while_m_n(1, 2, is_digit),
    )))(i)
}

fn date_only(i: &str) -> IResult<&str, Date> {
    map_res(date_only_raw, |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Date::Date))(i)
}

fn datetime(i: &str) -> IResult<&str, Date> {
    map_res(
        recognize(tuple((
            date_only_raw,
            char(' '),
            take_while_m_n(1, 2, is_digit),
            char(':'),
            take_while_m_n(1, 2, is_digit),
            char(':'),
            take_while_m_n(1, 2, is_digit),
        ))),
        |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(Date::Datetime),
    )(i)
}

fn date_hour(i: &str) -> IResult<&str, Date> {
    map_res(
        recognize(tuple((
            date_only_raw,
            char(' '),
            take_while_m_n(1, 2, is_digit),
            char(':'),
            take_while_m_n(1, 2, is_digit),
        ))),
        |s: &str| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").map(Date::DateHour),
    )(i)
}

/// `date = datetime | date_hour | date_only` — longest form first.
fn parse_date(i: &str) -> IResult<&str, Date> {
    alt((datetime, date_hour, date_only))(i)
}

// ---------------------------------------------------------------------------
// numbers and arithmetic expressions
// ---------------------------------------------------------------------------

/// A numeric literal that may contain `,`/`_` group separators, a fractional
/// part, and an optional scientific-notation exponent.
fn number(i: &str) -> IResult<&str, BigDecimal> {
    map_res(
        recognize(tuple((
            take_while1(is_digit),
            take_while(|c: char| is_digit(c) || matches!(c, ',' | '_')),
            opt(pair(char('.'), take_while(is_digit))),
            opt(tuple((one_of("eE"), opt(one_of("+-")), take_while1(is_digit)))),
        ))),
        |s: &str| BigDecimal::from_str(&s.replace([',', '_'], "")),
    )(i)
}

/// `expr_primary = number | "(" number_expr ")"`
fn expr_primary(i: &str) -> IResult<&str, BigDecimal> {
    alt((number, delimited(pair(char('('), space0), number_expr, pair(space0, char(')')))))(i)
}

/// `expr_atom = "-"? space* expr_primary`
fn expr_atom(i: &str) -> IResult<&str, BigDecimal> {
    let (i, negative) = opt(char('-'))(i)?;
    let (i, _) = space0(i)?;
    let (i, value) = expr_primary(i)?;
    Ok((i, if negative.is_some() { -value } else { value }))
}

/// A binary operator surrounded by optional whitespace.
fn binary_operator(operators: &'static str) -> impl Fn(&str) -> IResult<&str, char> {
    move |i| {
        let (i, _) = space0(i)?;
        let (i, operator) = one_of(operators)(i)?;
        let (i, _) = space0(i)?;
        Ok((i, operator))
    }
}

/// Multiplicative level, left-associative: `expr_atom (("*" | "/") expr_atom)*`.
fn mul_expr(i: &str) -> IResult<&str, BigDecimal> {
    let (mut i, mut acc) = expr_atom(i)?;
    while let Ok((next, operator)) = binary_operator("*/")(i) {
        let (next, rhs) = expr_atom(next)?;
        acc = if operator == '*' { acc * rhs } else { acc / rhs };
        i = next;
    }
    Ok((i, acc))
}

/// Additive level, left-associative: `mul_expr (("+" | "-") mul_expr)*`.
///
/// Together with [`mul_expr`] and [`expr_atom`] this reproduces the precedence of
/// the original pratt parser (`* /` bind tighter than `+ -`, unary minus binds
/// tightest).
fn number_expr(i: &str) -> IResult<&str, BigDecimal> {
    let (mut i, mut acc) = mul_expr(i)?;
    while let Ok((next, operator)) = binary_operator("+-")(i) {
        let (next, rhs) = mul_expr(next)?;
        acc = if operator == '+' { acc + rhs } else { acc - rhs };
        i = next;
    }
    Ok((i, acc))
}

// ---------------------------------------------------------------------------
// postings
// ---------------------------------------------------------------------------

/// `posting_amount = number_expr space* commodity_name`
fn posting_amount(i: &str) -> IResult<&str, Amount> {
    let (i, number) = number_expr(i)?;
    let (i, _) = space0(i)?;
    let (i, currency) = commodity_name(i)?;
    Ok((i, Amount::new(number, currency)))
}

/// The `{ ... }` cost block of a posting.
enum CostComponent {
    Date(Date),
    Label(String),
}

/// A `,`-separated component of a cost spec: an acquisition date or a lot label.
fn cost_component(i: &str) -> IResult<&str, CostComponent> {
    alt((
        map(parse_date, CostComponent::Date),
        map(quote_string, |label| CostComponent::Label(label.to_plain_string())),
    ))(i)
}

fn cost_group(i: &str) -> IResult<&str, PostingCost> {
    // `{{ }}` is a total cost, `{ }` is a per-unit cost.
    let (i, total) = alt((value(true, tag("{{")), value(false, char('{'))))(i)?;
    let (i, _) = space0(i)?;
    let (i, base) = opt(posting_amount)(i)?;
    let (i, components) = many0(preceded(tuple((space0, char(','), space0)), cost_component))(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = if total { value((), tag("}}"))(i)? } else { value((), char('}'))(i)? };

    let mut date = None;
    let mut label = None;
    for component in components {
        match component {
            CostComponent::Date(d) => date = Some(d),
            CostComponent::Label(l) => label = Some(l),
        }
    }
    Ok((i, PostingCost { base, date, label, total }))
}

/// `posting_price = "@@" ... | "@" ...`
fn posting_price(i: &str) -> IResult<&str, SingleTotalPrice> {
    alt((
        map(preceded(pair(tag("@@"), space0), posting_amount), SingleTotalPrice::Total),
        map(preceded(pair(char('@'), space0), posting_amount), SingleTotalPrice::Single),
    ))(i)
}

type PostingMeta = (Option<PostingCost>, Option<SingleTotalPrice>);

/// `posting_meta = ("{" ... "}")? space* posting_price?`
fn posting_meta(i: &str) -> IResult<&str, PostingMeta> {
    let (i, cost) = opt(preceded(space0, cost_group))(i)?;
    let (i, _) = space0(i)?;
    let (i, price) = opt(posting_price)(i)?;
    Ok((i, (cost, price)))
}

/// `posting_unit = posting_amount? posting_meta`
fn posting_unit(i: &str) -> IResult<&str, (Option<Amount>, Option<PostingMeta>)> {
    let (i, amount) = opt(posting_amount)(i)?;
    let (i, meta) = posting_meta(i)?;
    Ok((i, (amount, Some(meta))))
}

/// `transaction_flag = space+ ("!" | "*" | "#" | ASCII_ALPHA_UPPER)`
fn transaction_flag(i: &str) -> IResult<&str, Flag> {
    let (i, _) = space1(i)?;
    alt((
        // beancount's `txn` keyword is the explicit form of a completed transaction
        map(tag("txn"), |_| Flag::Okay),
        map(satisfy(|c: char| c == '!' || c == '*' || c == '#' || c.is_ascii_uppercase()), |c| {
            Flag::from_str(&c.to_string()).expect("invalid flag")
        }),
    ))(i)
}

/// `transaction_posting = transaction_flag? account_name (space+ posting_unit)?`
fn transaction_posting(i: &str) -> IResult<&str, Posting> {
    let (i, flag) = opt(transaction_flag)(i)?;
    let (i, account) = account_name(i)?;
    let (i, unit) = opt(preceded(space1, posting_unit))(i)?;

    let mut posting = Posting {
        flag,
        account,
        units: None,
        cost: None,
        price: None,
        comment: None,
    };
    if let Some((amount, meta)) = unit {
        posting.units = amount;
        if let Some((cost, price)) = meta {
            posting.cost = cost;
            posting.price = price;
        }
    }
    Ok((i, posting))
}

enum TransactionLine {
    Posting(Posting),
    Meta((String, ZhangString)),
}

/// A single indented line inside a transaction: a posting, a metadata pair, or an
/// (ignored) comment / blank line.
fn transaction_line(i: &str) -> IResult<&str, (Option<Posting>, Option<(String, ZhangString)>)> {
    let (i, _) = line_ending(i)?;
    let (i, _) = space1(i)?;
    let (i, content) = opt(alt((
        map(transaction_posting, TransactionLine::Posting),
        map(key_value_line, TransactionLine::Meta),
    )))(i)?;
    let (i, _) = space0(i)?;
    let (i, comment) = opt(valuable_comment_body)(i)?;

    let result = match content {
        Some(TransactionLine::Posting(posting)) => {
            let posting = match comment {
                Some(comment) => posting.set_comment(comment),
                None => posting,
            };
            (Some(posting), None)
        }
        Some(TransactionLine::Meta(meta)) => (None, Some(meta)),
        None => (None, None),
    };
    Ok((i, result))
}

/// `transaction_lines = transaction_line+`
fn transaction_lines(i: &str) -> IResult<&str, Vec<(Option<Posting>, Option<(String, ZhangString)>)>> {
    many1(transaction_line)(i)
}

/// A tag (`#name`) or link (`^name`) preceded by optional whitespace. The bool is
/// `true` for a tag, `false` for a link.
fn spaced_tag_or_link(i: &str) -> IResult<&str, (bool, String)> {
    preceded(
        space0,
        alt((
            map(preceded(char('#'), unquote_string_raw), |s: &str| (true, s.to_string())),
            map(preceded(char('^'), unquote_string_raw), |s: &str| (false, s.to_string())),
        )),
    )(i)
}

/// `tags_or_links = (space* (tag | link))*`
fn tags_or_links(i: &str) -> IResult<&str, (Vec<String>, Vec<String>)> {
    let mut tags = Vec::new();
    let mut links = Vec::new();
    let mut rest = i;
    while let Ok((next, (is_tag, value))) = spaced_tag_or_link(rest) {
        if is_tag {
            tags.push(value);
        } else {
            links.push(value);
        }
        rest = next;
    }
    Ok((rest, (tags, links)))
}

// ---------------------------------------------------------------------------
// metadata
// ---------------------------------------------------------------------------

/// `key_value_line = string space* ":" space* string`
fn key_value_line(i: &str) -> IResult<&str, (String, ZhangString)> {
    let (i, key) = string(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = char(':')(i)?;
    let (i, _) = space0(i)?;
    let (i, value) = string(i)?;
    Ok((i, (key.to_plain_string(), value)))
}

/// A single indented metadata line following a directive.
fn meta_line(i: &str) -> IResult<&str, (String, ZhangString)> {
    let (i, _) = line_ending(i)?;
    let (i, _) = space1(i)?;
    let (i, pair) = key_value_line(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = opt(inline_comment)(i)?;
    Ok((i, pair))
}

/// `metas = (line space+ key_value_line comment?)+`
fn metas_block(i: &str) -> IResult<&str, Meta> {
    map(many1(meta_line), |pairs| pairs.into_iter().collect())(i)
}

// ---------------------------------------------------------------------------
// directive bodies (everything after `date keyword`)
// ---------------------------------------------------------------------------

fn comma_separator(i: &str) -> IResult<&str, ()> {
    value((), tuple((space0, char(','), space0)))(i)
}

fn open_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, commodities) = opt(preceded(space1, separated_list1(comma_separator, commodity_name)))(i)?;
    Ok((
        i,
        Directive::Open(Open {
            date,
            account,
            commodities: commodities.unwrap_or_default(),
            meta: Meta::default(),
        }),
    ))
}

fn close_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    Ok((
        i,
        Directive::Close(Close {
            date,
            account,
            meta: Meta::default(),
        }),
    ))
}

fn note_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, comment) = string(i)?;
    Ok((
        i,
        Directive::Note(Note {
            date,
            account,
            comment,
            tags: None,
            links: None,
            meta: Meta::default(),
        }),
    ))
}

fn balance_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = number_expr(i)?;
    let (i, tolerance) = opt(preceded(tuple((space1, char('~'), space0)), number_expr))(i)?;
    let (i, _) = space1(i)?;
    let (i, commodity) = commodity_name(i)?;
    let (i, pad) = opt(preceded(tuple((space1, tag("with"), space1, tag("pad"), space1)), account_name))(i)?;

    let amount = Amount::new(amount, commodity);
    let directive = match pad {
        // a `~ tolerance` on a `with pad` balance is meaningless (pad makes it exact); drop it
        Some(pad) => Directive::BalancePad(BalancePad {
            date,
            account,
            amount,
            pad,
            meta: Meta::default(),
        }),
        None => Directive::BalanceCheck(BalanceCheck {
            date,
            account,
            amount,
            tolerance,
            meta: Meta::default(),
        }),
    };
    Ok((i, directive))
}

fn document_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, filename) = string(i)?;
    Ok((
        i,
        Directive::Document(Document {
            date,
            account,
            filename,
            tags: None,
            links: None,
            meta: Meta::default(),
        }),
    ))
}

fn price_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, currency) = commodity_name(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = number_expr(i)?;
    let (i, _) = space1(i)?;
    let (i, target) = commodity_name(i)?;
    Ok((
        i,
        Directive::Price(Price {
            date,
            currency,
            amount: Amount::new(amount, target),
            meta: Meta::default(),
        }),
    ))
}

fn event_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, event_type) = string(i)?;
    let (i, _) = space1(i)?;
    let (i, description) = string(i)?;
    Ok((
        i,
        Directive::Event(Event {
            date,
            event_type,
            description,
            meta: Meta::default(),
        }),
    ))
}

fn commodity_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, currency) = commodity_name(i)?;
    Ok((
        i,
        Directive::Commodity(Commodity {
            date,
            currency,
            meta: Meta::default(),
        }),
    ))
}

fn string_or_account(i: &str) -> IResult<&str, StringOrAccount> {
    alt((map(account_name, StringOrAccount::Account), map(string, StringOrAccount::String)))(i)
}

fn custom_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, custom_type) = string(i)?;
    let (i, values) = many1(preceded(space1, string_or_account))(i)?;
    Ok((
        i,
        Directive::Custom(Custom {
            date,
            custom_type,
            values,
            meta: Meta::default(),
        }),
    ))
}

fn budget_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, name) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, commodity) = commodity_name(i)?;
    Ok((
        i,
        Directive::Budget(Budget {
            date,
            name: name.to_string(),
            commodity,
            meta: Meta::default(),
        }),
    ))
}

fn budget_add_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, name) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = posting_amount(i)?;
    Ok((
        i,
        Directive::BudgetAdd(BudgetAdd {
            date,
            name: name.to_string(),
            amount,
            meta: Meta::default(),
        }),
    ))
}

fn budget_transfer_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, from) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, to) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = posting_amount(i)?;
    Ok((
        i,
        Directive::BudgetTransfer(BudgetTransfer {
            date,
            from: from.to_string(),
            to: to.to_string(),
            amount,
            meta: Meta::default(),
        }),
    ))
}

fn budget_close_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, name) = unquote_string_raw(i)?;
    Ok((
        i,
        Directive::BudgetClose(BudgetClose {
            date,
            name: name.to_string(),
            meta: Meta::default(),
        }),
    ))
}

/// A dated directive: parse the shared `date keyword` prefix, then dispatch on the
/// keyword. Fails (so the caller can try a transaction) when the keyword after the
/// date is unknown.
fn dated_directive(original: &str) -> IResult<&str, Directive> {
    let (i, date) = terminated(parse_date, space1)(original)?;
    let (rest, keyword) = take_while1(|c: char| c.is_ascii_lowercase() || c == '-')(i)?;
    match keyword {
        "open" => open_body(date, rest),
        "close" => close_body(date, rest),
        "note" => note_body(date, rest),
        "balance" => balance_body(date, rest),
        "document" => document_body(date, rest),
        "price" => price_body(date, rest),
        "event" => event_body(date, rest),
        "commodity" => commodity_body(date, rest),
        "custom" => custom_body(date, rest),
        "budget" => budget_body(date, rest),
        "budget-add" => budget_add_body(date, rest),
        "budget-transfer" => budget_transfer_body(date, rest),
        "budget-close" => budget_close_body(date, rest),
        _ => Err(nom::Err::Error(nom::error::Error::new(original, nom::error::ErrorKind::Tag))),
    }
}

/// `plugin = "plugin" space+ string (space+ string)*`
fn plugin_directive(i: &str) -> IResult<&str, Directive> {
    let (i, _) = tag("plugin")(i)?;
    let (i, _) = space1(i)?;
    let (i, module) = string(i)?;
    let (i, values) = many0(preceded(space1, string))(i)?;
    Ok((
        i,
        Directive::Plugin(Plugin {
            module,
            value: values,
            meta: Meta::default(),
        }),
    ))
}

/// `option = "option" space+ string space+ string`
fn option_directive(i: &str) -> IResult<&str, Directive> {
    let (i, _) = tag("option")(i)?;
    let (i, _) = space1(i)?;
    let (i, key) = string(i)?;
    let (i, _) = space1(i)?;
    let (i, value) = string(i)?;
    Ok((i, Directive::Option(Options { key, value })))
}

/// `include = "include" space+ quote_string`
fn include_directive(i: &str) -> IResult<&str, Directive> {
    let (i, _) = tag("include")(i)?;
    let (i, _) = space1(i)?;
    let (i, file) = quote_string(i)?;
    Ok((i, Directive::Include(Include { file })))
}

/// A `metable_head` (dated directive or plugin) plus an optional trailing comment
/// and metadata block.
fn metable_item(i: &str) -> IResult<&str, Directive> {
    let (i, directive) = alt((plugin_directive, dated_directive))(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = opt(inline_comment)(i)?;
    let (i, metas) = opt(metas_block)(i)?;
    let directive = match metas {
        Some(meta) => directive.set_meta(meta),
        None => directive,
    };
    Ok((i, directive))
}

/// `transaction = date flag? ("payee"? "narration"?) tags_or_links? comment? transaction_lines`
fn transaction(original: &str) -> IResult<&str, Directive> {
    let (i, date) = parse_date(original)?;
    let (i, flag) = opt(transaction_flag)(i)?;
    let (i, strings) = many_m_n(0, 2, preceded(space1, quote_string))(i)?;
    let (i, (tags, links)) = tags_or_links(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = opt(inline_comment)(i)?;
    let (i, lines) = transaction_lines(i)?;

    // A transaction must carry at least a flag or a quoted string, otherwise the
    // line is not a transaction at all.
    if flag.is_none() && strings.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(original, nom::error::ErrorKind::Verify)));
    }

    let count = strings.len();
    let mut strings = strings.into_iter();
    let (payee, narration) = match (flag.is_some(), count) {
        (_, 2) => (strings.next(), strings.next()),
        (false, 1) => (strings.next(), None),
        (true, 1) => (None, strings.next()),
        _ => (None, None),
    };

    let mut transaction = Transaction {
        date,
        flag,
        payee,
        narration,
        tags: tags.into_iter().collect(),
        links: links.into_iter().collect(),
        postings: Vec::new(),
        meta: Meta::default(),
    };
    for line in lines {
        match line {
            (Some(posting), None) => transaction.postings.push(posting),
            (None, Some((key, value))) => {
                transaction.meta.insert(key, value);
            }
            _ => {}
        }
    }
    Ok((i, Directive::Transaction(transaction)))
}

/// Parse one top-level item. Returns `None` for items that produce no directive
/// (currently only impossible-to-reach empty lines, kept for completeness).
fn content_item(i: &str) -> IResult<&str, Option<Directive>> {
    alt((
        map(terminated(option_directive, line_trailer), Some),
        map(terminated(include_directive, line_trailer), Some),
        map(valuable_comment, |content| Some(Directive::Comment(Comment { content }))),
        map(metable_item, Some),
        map(transaction, Some),
    ))(i)
}

fn error_at(original: &str, rest: &str, message: &str) -> ParseError {
    let position = offset(original, rest);
    let consumed = &original[..position];
    let line = consumed.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = position - consumed.rfind('\n').map(|index| index + 1).unwrap_or(0) + 1;
    ParseError {
        message: format!("failed to parse zhang file: {} at line {}, column {}", message, line, column),
    }
}

/// Parse a full zhang text file into a list of spanned directives.
pub fn parse(input_str: &str, file: impl Into<Option<PathBuf>>) -> Result<Vec<Spanned<Directive>>, ParseError> {
    let file = file.into();
    let original = input_str;
    let mut rest = input_str;
    let mut directives: Vec<Spanned<Directive>> = Vec::new();

    loop {
        while let Ok((next, _)) = blank_line(rest) {
            rest = next;
        }
        if rest.is_empty() || rest.bytes().all(|byte| byte == b' ' || byte == b'\t') {
            break;
        }

        let start = offset(original, rest);
        let (next, directive) = content_item(rest).map_err(|_| error_at(original, rest, "unexpected input"))?;

        // Defensive: every successful item must make progress.
        if offset(original, next) == start {
            return Err(error_at(original, rest, "parser made no progress"));
        }

        if let Some(directive) = directive {
            let end = offset(original, next);
            directives.push(Spanned {
                data: directive,
                span: SpanInfo {
                    start,
                    end,
                    content: original[start..end].to_string(),
                    filename: file.clone(),
                },
            });
        }
        rest = next;
    }

    Ok(directives)
}

#[cfg(test)]
mod test {
    use zhang_ast::{Directive, Transaction};

    use crate::data_type::text::parser::parse;
    macro_rules! quote {
        ($s: expr) => {
            zhang_ast::ZhangString::QuoteString($s.to_string())
        };
    }
    // macro_rules! unquote {
    //     ($s: expr) => {
    //         crate::core::models::ZhangString::UnquoteString($s.to_string())
    //     };
    // }
    macro_rules! date {
        ($year: expr,$month: expr, $day: expr) => {
            zhang_ast::Date::Date(chrono::NaiveDate::from_ymd_opt($year, $month, $day).unwrap())
        };
        ($year: expr,$month: expr, $day: expr,$hour: expr,$min: expr) => {
            zhang_ast::Date::DateHour(
                chrono::NaiveDate::from_ymd_opt($year, $month, $day)
                    .unwrap()
                    .and_hms_opt($hour, $min, 0)
                    .unwrap(),
            )
        };
        ($year: expr,$month: expr, $day: expr,$hour: expr,$min: expr,$sec: expr) => {
            zhang_ast::Date::Datetime(
                chrono::NaiveDate::from_ymd_opt($year, $month, $day)
                    .unwrap()
                    .and_hms_opt($hour, $min, $sec)
                    .unwrap(),
            )
        };
    }
    macro_rules! account {
        ($account: expr) => {{
            use std::str::FromStr;
            zhang_ast::account::Account::from_str($account).unwrap()
        }};
    }

    fn get_txn(content: &str) -> Transaction {
        let directive = parse(content, None).unwrap().pop().unwrap().data;
        match directive {
            Directive::Transaction(txn) => txn,
            _ => unreachable!("should get txn, but other directive is found"),
        }
    }

    mod date_time_support {

        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use zhang_ast::amount::Amount;
        use zhang_ast::*;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse_date_hour() {
            let mut result = parse("2101-10-10 10:10 open Assets:Hello", None).unwrap();
            let directive = result.remove(0);
            assert_eq!(
                Directive::Open(Open {
                    date: date!(2101, 10, 10, 10, 10),
                    account: account!("Assets:Hello"),
                    commodities: vec![],
                    meta: Default::default()
                }),
                directive.data
            )
        }

        #[test]
        fn should_parse_balance_check_and_balance_pad() {
            let balance = parse("2101-10-10 10:10 balance Assets:Hello 123 CNY", None).unwrap().remove(0);
            assert_eq!(
                Directive::BalanceCheck(BalanceCheck {
                    date: Date::DateHour(NaiveDate::from_ymd_opt(2101, 10, 10).unwrap().and_hms_opt(10, 10, 0).unwrap()),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    amount: Amount::new(BigDecimal::from(123i32), "CNY"),
                    tolerance: None,
                    meta: Default::default()
                }),
                balance.data
            );

            let balance = parse("2101-10-10 10:10 balance Assets:Hello 123 CNY with pad Income:Earnings", None)
                .unwrap()
                .remove(0);
            assert_eq!(
                Directive::BalancePad(BalancePad {
                    date: Date::DateHour(NaiveDate::from_ymd_opt(2101, 10, 10).unwrap().and_hms_opt(10, 10, 0).unwrap()),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    amount: Amount::new(BigDecimal::from(123i32), "CNY"),
                    pad: Account::from_str("Income:Earnings").unwrap(),
                    meta: Default::default()
                }),
                balance.data
            )
        }
    }
    mod options {

        use indoc::indoc;
        use zhang_ast::*;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            option "title" "Example"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            assert_eq!(
                vec.pop().unwrap().data,
                Directive::Option(Options {
                    key: quote!("title"),
                    value: quote!("Example")
                })
            );
        }
    }
    mod document {

        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 01:01:01 document Assets:Card "abc.jpg"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Document(..)));
            if let Directive::Document(inner) = directive {
                assert_eq!(inner.date, date!(1970, 1, 1, 1, 1, 1));
                assert_eq!(inner.account, account!("Assets:Card"));
                assert_eq!(inner.filename, quote!("abc.jpg"));
            }
        }
    }
    mod price {

        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 01:01:01 price USD 7 CNY
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Price(..)));
            if let Directive::Price(inner) = directive {
                assert_eq!(inner.date, date!(1970, 1, 1, 1, 1, 1));
                assert_eq!(inner.currency, "USD");
                assert_eq!(inner.amount.currency, "CNY");
                assert_eq!(inner.amount.number, BigDecimal::from(7i32));
            }
        }
    }
    mod event {

        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 01:01:01 event "something" "value"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Event(..)));
            if let Directive::Event(inner) = directive {
                assert_eq!(inner.date, date!(1970, 1, 1, 1, 1, 1));
                assert_eq!(inner.event_type, quote!("something"));
                assert_eq!(inner.description, quote!("value"));
            }
        }
    }
    mod plugin {

        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            plugin "module" "123" "345"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Plugin(..)));
            if let Directive::Plugin(inner) = directive {
                assert_eq!(inner.module, quote!("module"));
                assert_eq!(inner.value, vec![quote!("123"), quote!("345")]);
            }
        }

        #[test]
        fn should_support_meta() {
            let mut vec = parse(
                indoc! {r#"
                            plugin "module" "123" "345"
                              a: "b"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Plugin(..)));
            if let Directive::Plugin(inner) = directive {
                assert_eq!(inner.meta.get_one("a"), Some(&quote!("b")));
            }
        }
    }

    mod custom {

        use indoc::indoc;
        use zhang_ast::{Directive, StringOrAccount};

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 01:01:01 custom "budget" Assets:Card "100 CNY" "monthly"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Custom(..)));
            if let Directive::Custom(inner) = directive {
                assert_eq!(inner.date, date!(1970, 1, 1, 1, 1, 1));
                assert_eq!(inner.custom_type, quote!("budget"));
                assert_eq!(
                    inner.values,
                    vec![
                        StringOrAccount::Account(account!("Assets:Card")),
                        StringOrAccount::String(quote!("100 CNY")),
                        StringOrAccount::String(quote!("monthly"))
                    ]
                );
            }
        }
        #[test]
        fn should_parse_with_meta() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 01:01:01 custom "budget" Assets:Card "100 CNY" "monthly"
                              alias: "A"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Custom(..)));
            if let Directive::Custom(inner) = directive {
                assert_eq!(inner.meta.get_one("alias").unwrap(), &quote!("A"));
            }
        }
    }

    mod transaction {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use zhang_ast::{Directive, Flag};

        use crate::data_type::text::parser::parse;
        use crate::data_type::text::parser::test::get_txn;

        #[test]
        fn should_support_trailing_space() {
            let vec = parse(
                indoc! {r#"
                            2022-03-24 11:38:56 ""
                              Assets:B 1 CNY
                              Assets:B
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
        }

        #[test]
        fn should_support_arithmetic_in_postings() {
            let directive = parse(
                indoc! {r#"
                            2022-03-24 11:38:56 ""
                              Assets:B 120/10 + 1000 * (25--2) CNY
                              Assets:B
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data;
            match directive {
                Directive::Transaction(trx) => {
                    let posting = trx.postings.first().unwrap().clone();
                    assert_eq!(BigDecimal::from_str("27012").unwrap(), posting.units.unwrap().number)
                }
                _ => unreachable!("find other directives than txn directive"),
            }
        }

        #[test]
        fn should_support_any_upper_char_as_flag() {
            let trx = get_txn(indoc! {r#"
                2022-06-02 A "balanced transaction"
                  Assets:Card -1e-9 USD
                "#});
            assert_eq!(trx.flag, Some(Flag::Custom("A".to_string())));
        }
        #[test]
        fn should_support_hash_tag_as_flag() {
            let trx = get_txn(indoc! {r#"
                2022-06-02 # "balanced transaction"
                  Assets:Card -1e-9 USD
                "#});
            assert_eq!(trx.flag, Some(Flag::Custom("#".to_string())));
        }

        mod posting {
            use std::str::FromStr;

            use bigdecimal::BigDecimal;
            use chrono::NaiveDate;
            use indoc::indoc;
            use zhang_ast::amount::Amount;
            use zhang_ast::{Date, Directive, PostingCost, SingleTotalPrice, Transaction};

            use crate::data_type::text::parser::parse;

            fn get_first_posting(content: &str) -> Transaction {
                let directive = parse(content, None).unwrap().pop().unwrap();
                match directive.data {
                    Directive::Transaction(trx) => trx,
                    _ => unreachable!("find other directives than txn directive"),
                }
            }
            #[test]
            fn should_parse_multiple_postings() {
                let trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card
                  Expenses:Food
                "#});
                assert_eq!(2, trx.postings.len());
            }
            #[test]
            fn should_return_all_none_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(None, posting.units);
                assert_eq!(None, posting.cost);
                assert_eq!(None, posting.price);
            }

            #[test]
            fn should_return_unit() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 CNY
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "CNY")), posting.units);
                assert_eq!(None, posting.cost);
                assert_eq!(None, posting.price);
            }
            #[test]
            fn should_return_unit_and_cost() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD { 7 CNY }
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(
                    Some(PostingCost {
                        base: Some(Amount::new(BigDecimal::from(7i32), "CNY")),
                        date: None,
                        ..Default::default()
                    }),
                    posting.cost
                );
                assert_eq!(None, posting.price);
            }

            #[test]
            fn should_return_unit_and_cost_cost_date() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD { 7 CNY, 2022-06-06 }
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(
                    Some(PostingCost {
                        base: Some(Amount::new(BigDecimal::from(7i32), "CNY")),
                        date: Some(Date::Date(NaiveDate::from_ymd_opt(2022, 6, 6).unwrap())),
                        ..Default::default()
                    }),
                    posting.cost
                );
                assert_eq!(None, posting.price);
            }
            #[test]
            fn should_return_unit_and_single_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD @ 7 CNY
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(None, posting.cost);
                assert_eq!(Some(SingleTotalPrice::Single(Amount::new(BigDecimal::from(7i32), "CNY"))), posting.price);
            }
            #[test]
            fn should_return_unit_and_total_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD @@ 700 CNY
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(None, posting.cost);
                assert_eq!(Some(SingleTotalPrice::Total(Amount::new(BigDecimal::from(700i32), "CNY"))), posting.price);
            }
            #[test]
            fn should_return_unit_cost_and_single_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD { 6.9 CNY } @ 7 CNY
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(
                    Some(PostingCost {
                        base: Some(Amount::new(BigDecimal::from_str("6.9").unwrap(), "CNY")),
                        date: None,
                        ..Default::default()
                    }),
                    posting.cost
                );
                assert_eq!(Some(SingleTotalPrice::Single(Amount::new(BigDecimal::from(7i32), "CNY"))), posting.price);
            }
            #[test]
            fn should_support_implicit_cost() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD {  }
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(
                    Some(PostingCost {
                        base: None,
                        date: None,
                        ..Default::default()
                    }),
                    posting.cost
                );
                assert_eq!(None, posting.price);
            }
            #[test]
            fn should_support_implicit_cost_and_price() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD { } @ 7 CNY
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(Some(Amount::new(BigDecimal::from(-100i32), "USD")), posting.units);
                assert_eq!(
                    Some(PostingCost {
                        base: None,
                        date: None,
                        ..Default::default()
                    }),
                    posting.cost
                );
                assert_eq!(Some(SingleTotalPrice::Single(Amount::new(BigDecimal::from(7i32), "CNY"))), posting.price);
            }

            #[test]
            fn should_support_comma_char_for_human_readable_number() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1,000.00 USD
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(BigDecimal::from_str("-1000").unwrap(), posting.units.unwrap().number);
            }
            #[test]
            fn should_support_underline_char_for_human_readable_number() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1_000.00 USD
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(BigDecimal::from_str("-1000").unwrap(), posting.units.unwrap().number);
            }
            #[test]
            fn should_support_scientific_math() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1e9 USD
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(BigDecimal::from_str("-1000000000").unwrap(), posting.units.unwrap().number);
            }
            #[test]
            fn should_support_scientific_math_with_plus_symbol() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1e+9 USD
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(BigDecimal::from_str("-1000000000").unwrap(), posting.units.unwrap().number);
            }
            #[test]
            fn should_support_scientific_math_with_minus_symbol() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1e-9 USD
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!(BigDecimal::from_str("-0.000000001").unwrap(), posting.units.unwrap().number);
            }
        }
    }
    mod budget {
        use bigdecimal::{BigDecimal, One};
        use indoc::indoc;
        use zhang_ast::amount::Amount;
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse_budget_without_meta() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 budget Diet CNY
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Budget(..)));
            if let Directive::Budget(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.commodity, "CNY");
            }
        }

        #[test]
        fn should_parse_budget_with_meta() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 budget Diet CNY
                              alias: "日常饮食"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::Budget(..)));
            if let Directive::Budget(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.commodity, "CNY");
                assert_eq!(inner.meta.get_one("alias").unwrap(), &quote!("日常饮食"));
            }
        }

        #[test]
        fn should_parse_budget_add() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 budget-add Diet 1 CNY
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::BudgetAdd(..)));
            if let Directive::BudgetAdd(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.amount, Amount::new(BigDecimal::one(), "CNY".to_owned()));
            }
        }
        #[test]
        fn should_parse_budget_transfer() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 budget-transfer Diet Saving 1 CNY
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::BudgetTransfer(..)));
            if let Directive::BudgetTransfer(inner) = directive {
                assert_eq!(inner.from, "Diet");
                assert_eq!(inner.to, "Saving");
                assert_eq!(inner.amount, Amount::new(BigDecimal::one(), "CNY".to_owned()));
            }
        }

        #[test]
        fn should_parse_budget_close() {
            let mut vec = parse(
                indoc! {r#"
                            1970-01-01 budget-close Diet
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            assert!(matches!(directive, Directive::BudgetClose(..)));
            if let Directive::BudgetClose(inner) = directive {
                assert_eq!(inner.name, "Diet");
            }
        }
    }
}
