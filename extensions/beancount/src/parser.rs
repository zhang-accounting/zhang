//! Parser for the beancount text format.
//!
//! Hand-written recursive-descent parser built on [`nom`], replacing the previous
//! `pest` + `pest_consume` grammar (`beancount.pest`). It produces the same
//! [`BeancountDirective`] AST — the shared test module below is the behavioural
//! contract.
//!
//! beancount-only directives (`balance`, `pad`, `pushtag`, `poptag`) are returned
//! as [`Either::Right`]; everything else maps onto zhang's [`Directive`] as
//! [`Either::Left`].

use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveTime};
use itertools::Either;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while, take_while1, take_while_m_n};
use nom::character::complete::{char, line_ending, not_line_ending, one_of, satisfy, space0, space1};
use nom::combinator::{map, map_res, opt, peek, recognize, value};
use nom::multi::{many0, many1, many_m_n, separated_list1};
use nom::sequence::{delimited, pair, preceded, terminated, tuple};
use nom::IResult;
use snailquote::unescape;
use zhang_ast::amount::Amount;
use zhang_ast::*;

use crate::directives::{BalanceDirective, BeancountDirective, BeancountOnlyDirective, PadDirective};

/// Error returned when the input cannot be parsed as beancount text.
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

fn offset(original: &str, sub: &str) -> usize {
    sub.as_ptr() as usize - original.as_ptr() as usize
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

// ---------------------------------------------------------------------------
// low level tokens (shared shape with zhang-core's parser)
// ---------------------------------------------------------------------------

fn blank_line(i: &str) -> IResult<&str, ()> {
    value((), pair(space0, line_ending))(i)
}

fn comment_prefix(i: &str) -> IResult<&str, &str> {
    alt((tag("//"), tag(";"), tag("*"), tag("#")))(i)
}

fn inline_comment(i: &str) -> IResult<&str, ()> {
    value((), pair(comment_prefix, not_line_ending))(i)
}

fn line_trailer(i: &str) -> IResult<&str, ()> {
    value((), pair(space0, opt(inline_comment)))(i)
}

fn valuable_comment(i: &str) -> IResult<&str, String> {
    let (i, _) = space0(i)?;
    valuable_comment_body(i)
}

fn valuable_comment_body(i: &str) -> IResult<&str, String> {
    let (i, _) = comment_prefix(i)?;
    let (i, _) = space0(i)?;
    let (i, body) = not_line_ending(i)?;
    Ok((i, body.to_string()))
}

fn unquote_string_raw(i: &str) -> IResult<&str, &str> {
    take_while1(|c: char| !matches!(c, '"' | ':' | '(' | ')' | ',' | ' ' | '\t' | '\n' | '\r'))(i)
}

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

fn quote_string(i: &str) -> IResult<&str, ZhangString> {
    let (rest, raw) = recognize(tuple((char('"'), many0(string_char), char('"'))))(i)?;
    let unescaped = unescape(raw).expect("string contains invalid escape char");
    Ok((rest, ZhangString::QuoteString(unescaped)))
}

fn string(i: &str) -> IResult<&str, ZhangString> {
    alt((map(unquote_string_raw, |s: &str| ZhangString::UnquoteString(s.to_string())), quote_string))(i)
}

fn commodity_name(i: &str) -> IResult<&str, String> {
    map(
        recognize(pair(
            satisfy(|c: char| c.is_ascii_alphabetic()),
            take_while(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '\'')),
        )),
        |s: &str| s.to_string(),
    )(i)
}

fn account_type(i: &str) -> IResult<&str, &str> {
    alt((tag("Assets"), tag("Liabilities"), tag("Equity"), tag("Income"), tag("Expenses")))(i)
}

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

/// beancount dates are date-only; time (when present) is carried in metadata and
/// re-attached by the caller.
fn parse_date(i: &str) -> IResult<&str, Date> {
    map_res(
        recognize(tuple((
            take_while_m_n(4, 4, is_digit),
            char('-'),
            take_while_m_n(1, 2, is_digit),
            char('-'),
            take_while_m_n(1, 2, is_digit),
        ))),
        |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Date::Date),
    )(i)
}

// ---------------------------------------------------------------------------
// numbers and arithmetic expressions
// ---------------------------------------------------------------------------

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

fn expr_primary(i: &str) -> IResult<&str, BigDecimal> {
    alt((number, delimited(pair(char('('), space0), number_expr, pair(space0, char(')')))))(i)
}

fn expr_atom(i: &str) -> IResult<&str, BigDecimal> {
    let (i, negative) = opt(char('-'))(i)?;
    let (i, _) = space0(i)?;
    let (i, value) = expr_primary(i)?;
    Ok((i, if negative.is_some() { -value } else { value }))
}

fn binary_operator(operators: &'static str) -> impl Fn(&str) -> IResult<&str, char> {
    move |i| {
        let (i, _) = space0(i)?;
        let (i, operator) = one_of(operators)(i)?;
        let (i, _) = space0(i)?;
        Ok((i, operator))
    }
}

fn mul_expr(i: &str) -> IResult<&str, BigDecimal> {
    let (mut i, mut acc) = expr_atom(i)?;
    while let Ok((next, operator)) = binary_operator("*/")(i) {
        let (next, rhs) = expr_atom(next)?;
        acc = if operator == '*' { acc * rhs } else { acc / rhs };
        i = next;
    }
    Ok((i, acc))
}

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

fn posting_amount(i: &str) -> IResult<&str, Amount> {
    let (i, number) = number_expr(i)?;
    let (i, _) = space0(i)?;
    let (i, currency) = commodity_name(i)?;
    Ok((i, Amount::new(number, currency)))
}

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

fn posting_price(i: &str) -> IResult<&str, SingleTotalPrice> {
    alt((
        map(preceded(pair(tag("@@"), space0), posting_amount), SingleTotalPrice::Total),
        map(preceded(pair(char('@'), space0), posting_amount), SingleTotalPrice::Single),
    ))(i)
}

type PostingMeta = (Option<PostingCost>, Option<SingleTotalPrice>);

fn posting_meta(i: &str) -> IResult<&str, PostingMeta> {
    let (i, cost) = opt(preceded(space0, cost_group))(i)?;
    let (i, _) = space0(i)?;
    let (i, price) = opt(posting_price)(i)?;
    Ok((i, (cost, price)))
}

fn posting_unit(i: &str) -> IResult<&str, (Option<Amount>, Option<PostingMeta>)> {
    let (i, amount) = opt(posting_amount)(i)?;
    let (i, meta) = posting_meta(i)?;
    Ok((i, (amount, Some(meta))))
}

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

fn transaction_lines(i: &str) -> IResult<&str, Vec<(Option<Posting>, Option<(String, ZhangString)>)>> {
    many1(transaction_line)(i)
}

fn spaced_tag_or_link(i: &str) -> IResult<&str, (bool, String)> {
    preceded(
        space0,
        alt((
            map(preceded(char('#'), unquote_string_raw), |s: &str| (true, s.to_string())),
            map(preceded(char('^'), unquote_string_raw), |s: &str| (false, s.to_string())),
        )),
    )(i)
}

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

fn key_value_line(i: &str) -> IResult<&str, (String, ZhangString)> {
    let (i, key) = string(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = char(':')(i)?;
    let (i, _) = space0(i)?;
    let (i, value) = string(i)?;
    Ok((i, (key.to_plain_string(), value)))
}

fn meta_line(i: &str) -> IResult<&str, (String, ZhangString)> {
    let (i, _) = line_ending(i)?;
    let (i, _) = space1(i)?;
    let (i, pair) = key_value_line(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = opt(inline_comment)(i)?;
    Ok((i, pair))
}

fn metas_block(i: &str) -> IResult<&str, Meta> {
    map(many1(meta_line), |pairs| pairs.into_iter().collect())(i)
}

// ---------------------------------------------------------------------------
// directive bodies
// ---------------------------------------------------------------------------

fn comma_separator(i: &str) -> IResult<&str, ()> {
    value((), tuple((space0, char(','), space0)))(i)
}

/// `booking_method = "\"" ("STRICT" | "FIFO" | "LIFO" | "AVERAGE" | "AVERAGE_ONLY" | "NONE") "\""`
fn booking_method(i: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        map(
            alt((tag("STRICT"), tag("FIFO"), tag("LIFO"), tag("AVERAGE_ONLY"), tag("AVERAGE"), tag("NONE"))),
            |s: &str| s.to_string(),
        ),
        char('"'),
    )(i)
}

fn open_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, commodities) = opt(preceded(space1, separated_list1(comma_separator, commodity_name)))(i)?;
    let (i, booking) = opt(preceded(space1, booking_method))(i)?;

    let mut meta = Meta::default();
    if let Some(booking) = booking {
        meta.insert("booking_method".to_string(), ZhangString::quote(booking));
    }
    Ok((
        i,
        Either::Left(Directive::Open(Open {
            date,
            account,
            commodities: commodities.unwrap_or_default(),
            meta,
        })),
    ))
}

fn close_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    Ok((
        i,
        Either::Left(Directive::Close(Close {
            date,
            account,
            meta: Meta::default(),
        })),
    ))
}

fn note_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, comment) = string(i)?;
    Ok((
        i,
        Either::Left(Directive::Note(Note {
            date,
            account,
            comment,
            tags: None,
            links: None,
            meta: Meta::default(),
        })),
    ))
}

fn balance_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = number_expr(i)?;
    let (i, tolerance) = opt(preceded(tuple((space1, char('~'), space0)), number_expr))(i)?;
    let (i, _) = space1(i)?;
    let (i, commodity) = commodity_name(i)?;
    Ok((
        i,
        Either::Right(BeancountOnlyDirective::Balance(BalanceDirective {
            date,
            account,
            amount: Amount::new(amount, commodity),
            tolerance,
            meta: Meta::default(),
        })),
    ))
}

fn pad_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, pad) = account_name(i)?;
    Ok((
        i,
        Either::Right(BeancountOnlyDirective::Pad(PadDirective {
            date,
            account,
            pad,
            meta: Meta::default(),
        })),
    ))
}

fn document_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, filename) = string(i)?;
    Ok((
        i,
        Either::Left(Directive::Document(Document {
            date,
            account,
            filename,
            tags: None,
            links: None,
            meta: Meta::default(),
        })),
    ))
}

fn price_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, currency) = commodity_name(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = number_expr(i)?;
    let (i, _) = space1(i)?;
    let (i, target) = commodity_name(i)?;
    Ok((
        i,
        Either::Left(Directive::Price(Price {
            date,
            currency,
            amount: Amount::new(amount, target),
            meta: Meta::default(),
        })),
    ))
}

fn event_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, event_type) = string(i)?;
    let (i, _) = space1(i)?;
    let (i, description) = string(i)?;
    Ok((
        i,
        Either::Left(Directive::Event(Event {
            date,
            event_type,
            description,
            meta: Meta::default(),
        })),
    ))
}

fn commodity_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, currency) = commodity_name(i)?;
    Ok((
        i,
        Either::Left(Directive::Commodity(Commodity {
            date,
            currency,
            meta: Meta::default(),
        })),
    ))
}

fn string_or_account(i: &str) -> IResult<&str, StringOrAccount> {
    alt((map(account_name, StringOrAccount::Account), map(string, StringOrAccount::String)))(i)
}

/// `custom` directives. `custom budget ...` (and its `budget-add` / `budget-transfer`
/// / `budget-close` variants) become budget directives; anything else is a generic
/// custom directive.
fn custom_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let subtype = peek(unquote_string_raw)(i).ok().map(|(_, word)| word);
    match subtype {
        Some("budget") => budget_body(date, i),
        Some("budget-add") => budget_add_body(date, i),
        Some("budget-transfer") => budget_transfer_body(date, i),
        Some("budget-close") => budget_close_body(date, i),
        _ => {
            let (i, custom_type) = string(i)?;
            let (i, values) = many1(preceded(space1, string_or_account))(i)?;
            Ok((
                i,
                Either::Left(Directive::Custom(Custom {
                    date,
                    custom_type,
                    values,
                    meta: Meta::default(),
                })),
            ))
        }
    }
}

fn budget_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("budget")(i)?;
    let (i, _) = space1(i)?;
    let (i, name) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, commodity) = commodity_name(i)?;
    Ok((
        i,
        Either::Left(Directive::Budget(Budget {
            date,
            name: name.to_string(),
            commodity,
            meta: Meta::default(),
        })),
    ))
}

fn budget_add_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("budget-add")(i)?;
    let (i, _) = space1(i)?;
    let (i, name) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = posting_amount(i)?;
    Ok((
        i,
        Either::Left(Directive::BudgetAdd(BudgetAdd {
            date,
            name: name.to_string(),
            amount,
            meta: Meta::default(),
        })),
    ))
}

fn budget_transfer_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("budget-transfer")(i)?;
    let (i, _) = space1(i)?;
    let (i, from) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, to) = unquote_string_raw(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = posting_amount(i)?;
    Ok((
        i,
        Either::Left(Directive::BudgetTransfer(BudgetTransfer {
            date,
            from: from.to_string(),
            to: to.to_string(),
            amount,
            meta: Meta::default(),
        })),
    ))
}

fn budget_close_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("budget-close")(i)?;
    let (i, _) = space1(i)?;
    let (i, name) = unquote_string_raw(i)?;
    Ok((
        i,
        Either::Left(Directive::BudgetClose(BudgetClose {
            date,
            name: name.to_string(),
            meta: Meta::default(),
        })),
    ))
}

/// A dated directive: parse the shared `date keyword` prefix then dispatch on the
/// keyword. Fails (so the caller can try a transaction) on an unknown keyword.
fn dated_directive(original: &str) -> IResult<&str, BeancountDirective> {
    let (i, date) = terminated(parse_date, space1)(original)?;
    let (rest, keyword) = take_while1(|c: char| c.is_ascii_lowercase())(i)?;
    match keyword {
        "open" => open_body(date, rest),
        "close" => close_body(date, rest),
        "note" => note_body(date, rest),
        "balance" => balance_body(date, rest),
        "pad" => pad_body(date, rest),
        "document" => document_body(date, rest),
        "price" => price_body(date, rest),
        "event" => event_body(date, rest),
        "commodity" => commodity_body(date, rest),
        "custom" => custom_body(date, rest),
        _ => Err(nom::Err::Error(nom::error::Error::new(original, nom::error::ErrorKind::Tag))),
    }
}

fn plugin_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("plugin")(i)?;
    let (i, _) = space1(i)?;
    let (i, module) = string(i)?;
    let (i, values) = many0(preceded(space1, string))(i)?;
    Ok((
        i,
        Either::Left(Directive::Plugin(Plugin {
            module,
            value: values,
            meta: Meta::default(),
        })),
    ))
}

fn option_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("option")(i)?;
    let (i, _) = space1(i)?;
    let (i, key) = string(i)?;
    let (i, _) = space1(i)?;
    let (i, value) = string(i)?;
    Ok((i, Either::Left(Directive::Option(Options { key, value }))))
}

fn include_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("include")(i)?;
    let (i, _) = space1(i)?;
    let (i, file) = quote_string(i)?;
    Ok((i, Either::Left(Directive::Include(Include { file }))))
}

fn push_tag_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("pushtag")(i)?;
    let (i, _) = space1(i)?;
    let (i, _) = char('#')(i)?;
    let (i, name) = unquote_string_raw(i)?;
    Ok((i, Either::Right(BeancountOnlyDirective::PushTag(name.to_string()))))
}

fn pop_tag_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("poptag")(i)?;
    let (i, _) = space1(i)?;
    let (i, _) = char('#')(i)?;
    let (i, name) = unquote_string_raw(i)?;
    Ok((i, Either::Right(BeancountOnlyDirective::PopTag(name.to_string()))))
}

/// `pushmeta key: value` — push a metadata pair applied to following directives.
fn push_meta_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("pushmeta")(i)?;
    let (i, _) = space1(i)?;
    let (i, (key, value)) = key_value_line(i)?;
    Ok((i, Either::Right(BeancountOnlyDirective::PushMeta(key, value))))
}

/// `popmeta key:` — pop the most recently pushed metadata pair for `key`.
fn pop_meta_directive(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = tag("popmeta")(i)?;
    let (i, _) = space1(i)?;
    let (i, key) = string(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = char(':')(i)?;
    Ok((i, Either::Right(BeancountOnlyDirective::PopMeta(key.to_plain_string()))))
}

fn set_meta(directive: BeancountDirective, meta: Meta) -> BeancountDirective {
    match directive {
        Either::Left(directive) => Either::Left(directive.set_meta(meta)),
        Either::Right(directive) => Either::Right(directive.set_meta(meta)),
    }
}

fn metable_item(i: &str) -> IResult<&str, BeancountDirective> {
    let (i, directive) = alt((plugin_directive, dated_directive))(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = opt(inline_comment)(i)?;
    let (i, metas) = opt(metas_block)(i)?;
    let directive = match metas {
        Some(meta) => set_meta(directive, meta),
        None => directive,
    };
    Ok((i, directive))
}

fn transaction(original: &str) -> IResult<&str, BeancountDirective> {
    let (i, date) = parse_date(original)?;
    let (i, flag) = opt(transaction_flag)(i)?;
    let (i, strings) = many_m_n(0, 2, preceded(space1, quote_string))(i)?;
    let (i, (tags, links)) = tags_or_links(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = opt(inline_comment)(i)?;
    let (i, lines) = transaction_lines(i)?;

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
    Ok((i, Either::Left(Directive::Transaction(transaction))))
}

fn content_item(i: &str) -> IResult<&str, Option<BeancountDirective>> {
    alt((
        map(terminated(option_directive, line_trailer), Some),
        map(terminated(include_directive, line_trailer), Some),
        map(terminated(push_tag_directive, line_trailer), Some),
        map(terminated(pop_tag_directive, line_trailer), Some),
        map(terminated(push_meta_directive, line_trailer), Some),
        map(terminated(pop_meta_directive, line_trailer), Some),
        map(valuable_comment, |content| Some(Either::Left(Directive::Comment(Comment { content })))),
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
        message: format!("failed to parse beancount file: {} at line {}, column {}", message, line, column),
    }
}

/// Parse a full beancount text file into a list of spanned directives.
pub fn parse(input_str: &str, file: impl Into<Option<PathBuf>>) -> Result<Vec<Spanned<BeancountDirective>>, ParseError> {
    let file = file.into();
    let original = input_str;
    let mut rest = input_str;
    let mut directives: Vec<Spanned<BeancountDirective>> = Vec::new();

    loop {
        while let Ok((next, _)) = blank_line(rest) {
            rest = next;
        }
        if rest.is_empty() || rest.bytes().all(|byte| byte == b' ' || byte == b'\t') {
            break;
        }

        let start = offset(original, rest);
        let (next, directive) = content_item(rest).map_err(|_| error_at(original, rest, "unexpected input"))?;

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

/// Parse a `HH:MM:SS` time string, used to lift the `time:` metadata key onto a
/// directive's date.
pub fn parse_time(input_str: &str) -> Result<NaiveTime, ParseError> {
    let invalid = || ParseError {
        message: format!("invalid time: {}", input_str),
    };
    let parts: Vec<&str> = input_str.trim().split(':').collect();
    if parts.len() != 3 {
        return Err(invalid());
    }
    let hour = parts[0].parse::<u32>().map_err(|_| invalid())?;
    let minute = parts[1].parse::<u32>().map_err(|_| invalid())?;
    let second = parts[2].parse::<u32>().map_err(|_| invalid())?;
    NaiveTime::from_hms_opt(hour, minute, second).ok_or_else(invalid)
}

#[cfg(test)]
mod test {
    use zhang_ast::{Directive, Transaction};

    use crate::directives::BeancountOnlyDirective;
    use crate::parser::parse;

    fn get_left_directive(content: &str) -> Directive {
        parse(content, None).unwrap().pop().unwrap().data.left().unwrap()
    }
    fn get_txn(content: &str) -> Transaction {
        let directive = parse(content, None).unwrap().pop().unwrap().data.left().unwrap();
        match directive {
            Directive::Transaction(txn) => txn,
            _ => unreachable!("should get txn, but other directive is found"),
        }
    }
    fn get_right_directive(content: &str) -> BeancountOnlyDirective {
        parse(content, None).unwrap().pop().unwrap().data.right().unwrap()
    }
    mod tag {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Account, Date};

        use crate::directives::{BalanceDirective, BeancountOnlyDirective, PadDirective};
        use crate::parser::test::get_right_directive;

        #[test]
        fn should_support_push_tag() {
            let directive = get_right_directive("pushtag #mytag");
            assert_eq!(BeancountOnlyDirective::PushTag("mytag".to_string()), directive);
        }
        #[test]
        fn should_support_pop_tag() {
            let directive = get_right_directive("poptag #mytag");
            assert_eq!(BeancountOnlyDirective::PopTag("mytag".to_string()), directive);
        }

        #[test]
        fn should_parse_balance() {
            let directive = get_right_directive("1970-01-01 balance Assets:BankAccount 2 CNY");
            assert_eq!(
                BeancountOnlyDirective::Balance(BalanceDirective {
                    date: Date::Date(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
                    account: Account::from_str("Assets:BankAccount").unwrap(),
                    amount: Amount::new(BigDecimal::from(2i32), "CNY"),
                    tolerance: None,
                    meta: Default::default(),
                }),
                directive
            );
        }
        #[test]
        fn should_parse_pad() {
            let directive = get_right_directive("1970-01-01 pad Assets:BankAccount Assets:BankAccount2");
            assert_eq!(
                BeancountOnlyDirective::Pad(PadDirective {
                    date: Date::Date(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
                    account: Account::from_str("Assets:BankAccount").unwrap(),
                    pad: Account::from_str("Assets:BankAccount2").unwrap(),
                    meta: Default::default(),
                }),
                directive
            );
        }
    }
    mod txn {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use zhang_ast::Flag;

        use crate::parser::test::get_txn;

        #[test]
        fn should_parse_with_comment() {
            let directive = get_txn(indoc! {r#"
                            1970-01-01 "Payee" "Narration" ; 123123
                              Assets:Bank
                                a: b
                              Assets:Bank ;123213
                              a: b
                              b: c ;123123
                        "#});
            assert_eq!(directive.postings.get(1).unwrap().comment.as_ref().unwrap(), "123213");
        }

        #[test]
        fn should_support_arithmetic_expression_in_amount() {
            use indoc::indoc;
            let directive = get_txn(indoc! {r#"
                            1970-01-01 "Payee" "Narration"
                              Assets:Bank -(120/10) + 1000 * (25--2) CNY
                        "#});
            assert_eq!(directive.postings.first().unwrap().to_owned().units.unwrap().number, BigDecimal::from(26988));
        }
        #[test]
        fn should_support_comma_char_for_human_readable_number() {
            let mut trx = get_txn(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1,000.00 USD
                "#});
            let posting = trx.postings.pop().unwrap();
            assert_eq!(BigDecimal::from_str("-1000").unwrap(), posting.units.unwrap().number);
        }
        #[test]
        fn should_support_underline_char_for_human_readable_number() {
            let mut trx = get_txn(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1_000.00 USD
                "#});
            let posting = trx.postings.pop().unwrap();
            assert_eq!(BigDecimal::from_str("-1000").unwrap(), posting.units.unwrap().number);
        }
        #[test]
        fn should_support_scientific_math() {
            let mut trx = get_txn(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1e9 USD
                "#});
            let posting = trx.postings.pop().unwrap();
            assert_eq!(BigDecimal::from_str("-1000000000").unwrap(), posting.units.unwrap().number);
        }
        #[test]
        fn should_support_scientific_math_with_plus_symbol() {
            let mut trx = get_txn(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1e+9 USD
                "#});
            let posting = trx.postings.pop().unwrap();
            assert_eq!(BigDecimal::from_str("-1000000000").unwrap(), posting.units.unwrap().number);
        }
        #[test]
        fn should_support_scientific_math_with_minus_symbol() {
            let mut trx = get_txn(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -1e-9 USD
                "#});
            let posting = trx.postings.pop().unwrap();
            assert_eq!(BigDecimal::from_str("-0.000000001").unwrap(), posting.units.unwrap().number);
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
    }
    mod budget {
        use bigdecimal::{BigDecimal, One};
        use indoc::indoc;
        use zhang_ast::amount::Amount;
        use zhang_ast::Directive;

        use crate::parser::test::get_left_directive;

        #[test]
        fn should_parse_budget_without_meta() {
            let directive = get_left_directive(indoc! {r#"
                            1970-01-01 custom budget Diet CNY
                        "#});
            assert!(matches!(directive, Directive::Budget(..)));
            if let Directive::Budget(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.commodity, "CNY");
            }
        }

        #[test]
        fn should_parse_budget_with_meta() {
            let directive = get_left_directive(indoc! {r#"
                            1970-01-01 custom budget Diet CNY
                              alias: "日常饮食"
                        "#});
            assert!(matches!(directive, Directive::Budget(..)));
            if let Directive::Budget(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.commodity, "CNY");
                assert_eq!(inner.meta.get_one("alias").unwrap().clone().to_plain_string(), "日常饮食");
            }
        }

        #[test]
        fn should_parse_budget_add() {
            let directive = get_left_directive(indoc! {r#"
                            1970-01-01 custom budget-add Diet 1 CNY
                        "#});
            assert!(matches!(directive, Directive::BudgetAdd(..)));
            if let Directive::BudgetAdd(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.amount, Amount::new(BigDecimal::one(), "CNY".to_owned()));
            }
        }
        #[test]
        fn should_parse_budget_transfer() {
            let directive = get_left_directive(indoc! {r#"
                            1970-01-01 custom budget-transfer Diet Saving 1 CNY
                        "#});
            assert!(matches!(directive, Directive::BudgetTransfer(..)));
            if let Directive::BudgetTransfer(inner) = directive {
                assert_eq!(inner.from, "Diet");
                assert_eq!(inner.to, "Saving");
                assert_eq!(inner.amount, Amount::new(BigDecimal::one(), "CNY".to_owned()));
            }
        }

        #[test]
        fn should_parse_budget_close() {
            let directive = get_left_directive(indoc! {r#"
                            1970-01-01 custom budget-close Diet
                        "#});
            assert!(matches!(directive, Directive::BudgetClose(..)));
            if let Directive::BudgetClose(inner) = directive {
                assert_eq!(inner.name, "Diet");
            }
        }
    }
    mod single_line_item {
        mod options {
            use indoc::indoc;
            use zhang_ast::Directive;

            use crate::parser::test::get_left_directive;

            #[test]
            fn should_parse() {
                let directive = get_left_directive(indoc! {r#"
                            option "title" "Accounting"
                        "#});
                assert!(matches!(directive, Directive::Option(..)));
                if let Directive::Option(inner) = directive {
                    assert_eq!(inner.key.as_str(), "title");
                    assert_eq!(inner.value.as_str(), "Accounting");
                }
            }

            #[test]
            fn should_parse_with_comment() {
                let directive = get_left_directive(indoc! {r#"
                            option "title" "Accounting" ;123
                        "#});
                assert!(matches!(directive, Directive::Option(..)));
                if let Directive::Option(inner) = directive {
                    assert_eq!(inner.key.as_str(), "title");
                    assert_eq!(inner.value.as_str(), "Accounting");
                }
            }
        }

        mod open {
            use indoc::indoc;
            use zhang_ast::Directive;

            use crate::parser::test::get_left_directive;

            #[test]
            fn should_parse_with_booking_method() {
                let directive = get_left_directive(indoc! {r#"
                            1970-01-01 open Assets:Card CNY       "NONE"
                        "#});
                assert!(matches!(directive, Directive::Open(..)));
                if let Directive::Open(inner) = directive {
                    assert_eq!(inner.meta.get_one("booking_method").unwrap().as_str(), "NONE");
                }
            }
        }
    }
}
