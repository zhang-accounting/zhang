//! Parser for zhang's native text format.
//!
//! This is a hand-written recursive-descent parser built on [`nom`]. It replaces
//! the previous `pest` + `pest_consume` grammar (`zhang.pest`) while producing
//! exactly the same [`Directive`] AST — the shared test module below is the
//! behavioural contract.
//!
//! The `pub` token-level parsers are shared with the beancount data type, which
//! imports them instead of keeping copies.

use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while, take_while1, take_while_m_n};
use nom::character::complete::{char, line_ending, not_line_ending, one_of, satisfy, space0, space1};
use nom::combinator::{eof, map, map_res, opt, peek, recognize, value, verify};
use nom::multi::{many0, many1, many_m_n, separated_list1};
use nom::sequence::{delimited, pair, preceded, terminated, tuple};
use nom::IResult;
use zhang_ast::amount::Amount;
use zhang_ast::*;

use crate::utils::string_::{invalid_escape_at, quoted_string};
use crate::utils::BOM;

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
pub fn offset(original: &str, sub: &str) -> usize {
    sub.as_ptr() as usize - original.as_ptr() as usize
}

pub fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}

// ---------------------------------------------------------------------------
// low level tokens
// ---------------------------------------------------------------------------

/// A whitespace-only line together with its terminating newline.
pub fn blank_line(i: &str) -> IResult<&str, ()> {
    value((), pair(space0, line_ending))(i)
}

/// `comment_prefix = ";" | "#" | "//"`: the start of a comment in a zhang file. `*` is not one: it
/// is a flag, of a transaction and, before an account, of a posting ([`posting_flag`]), and a line
/// starting with `*` that is neither is an error, never a comment. The beancount parser has its own
/// prefixes, `*` among them for the org-mode headings beancount ignores.
fn comment_prefix(i: &str) -> IResult<&str, &str> {
    alt((tag("//"), tag(";"), tag("#")))(i)
}

/// An inline comment (prefix + rest of line), the whole of which is discarded.
pub fn inline_comment(i: &str) -> IResult<&str, ()> {
    value((), pair(comment_prefix, not_line_ending))(i)
}

/// Trailing `space* comment?` allowed after a single-line directive.
pub fn line_trailer(i: &str) -> IResult<&str, ()> {
    value((), pair(space0, opt(inline_comment)))(i)
}

/// `valuable_comment = space* comment_prefix space* comment_value`, returning the
/// comment body (`comment_value`).
pub fn valuable_comment(i: &str) -> IResult<&str, String> {
    let (i, _) = space0(i)?;
    valuable_comment_body(i)
}

/// The `comment_prefix space* comment_value` portion, assuming any leading spaces
/// are already consumed.
pub fn valuable_comment_body(i: &str) -> IResult<&str, String> {
    let (i, _) = comment_prefix(i)?;
    let (i, _) = space0(i)?;
    let (i, body) = not_line_ending(i)?;
    Ok((i, body.to_string()))
}

/// `unquote_string`: a bare word terminated by whitespace, quote, colon, paren or
/// comma.
pub fn unquote_string_raw(i: &str) -> IResult<&str, &str> {
    take_while1(|c: char| !matches!(c, '"' | ':' | '(' | ')' | ',' | ' ' | '\t' | '\n' | '\r'))(i)
}

/// `quote_string = "\"" inner "\""`, decoded by [`quoted_string`]: only `\"` and
/// `\\` must be escaped, unknown escapes such as `\d` are kept verbatim and the
/// escapes older zhang versions wrote (`\$`, `` \` ``, `\u{a0}`) are still read. A
/// malformed `\u` escape is a [`nom::Err::Failure`] at its backslash. See
/// [`crate::utils::string_`] for the full rules.
pub fn quote_string(i: &str) -> IResult<&str, ZhangString> {
    map(quoted_string, ZhangString::QuoteString)(i)
}

/// `string = unquote_string | quote_string`
pub fn string(i: &str) -> IResult<&str, ZhangString> {
    alt((map(unquote_string_raw, |s: &str| ZhangString::UnquoteString(s.to_string())), quote_string))(i)
}

/// `commodity_name = ASCII_ALPHA (ASCII_ALPHANUMERIC | "." | "_" | "-" | "'")*`
pub fn commodity_name(i: &str) -> IResult<&str, String> {
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
pub fn account_name(i: &str) -> IResult<&str, Account> {
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
pub fn number_expr(i: &str) -> IResult<&str, BigDecimal> {
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
pub fn posting_amount(i: &str) -> IResult<&str, Amount> {
    let (i, number) = number_expr(i)?;
    let (i, _) = space0(i)?;
    let (i, currency) = commodity_name(i)?;
    Ok((i, Amount::new(number, currency)))
}

/// A component of the `{ ... }` cost spec of a posting.
enum CostComponent {
    /// `150 USD`: the cost number, per unit or in total
    Amount(Amount),
    /// `100 # 5 USD`, a compound cost: the per-unit part and the total part, in one commodity
    Compound(Amount, BigDecimal),
    Date(Date),
    Label(String),
    /// `*`, beancount's merge-cost marker
    Merge,
}

/// `cost_amount = number_expr space* ("#" space* number_expr space*)? commodity_name`: the cost
/// number with its commodity, or beancount's compound cost `P # T USD` of a per-unit part and a
/// total part.
fn cost_amount(i: &str) -> IResult<&str, CostComponent> {
    let (i, number) = number_expr(i)?;
    let (i, _) = space0(i)?;
    let (i, total) = opt(delimited(pair(char('#'), space0), number_expr, space0))(i)?;
    let (i, currency) = commodity_name(i)?;
    let amount = Amount::new(number, currency);
    Ok((
        i,
        match total {
            Some(total) => CostComponent::Compound(amount, total),
            None => CostComponent::Amount(amount),
        },
    ))
}

/// `cost_component = date | cost_amount | string | "*"`: one `,`-separated component of a cost
/// spec, read with `date` as the date grammar of the format. A date comes first: its digits would
/// otherwise read as an arithmetic expression.
fn cost_component(date: fn(&str) -> IResult<&str, Date>) -> impl FnMut(&str) -> IResult<&str, CostComponent> {
    move |i| {
        alt((
            map(date, CostComponent::Date),
            cost_amount,
            map(quote_string, |label| CostComponent::Label(label.to_plain_string())),
            map(char('*'), |_| CostComponent::Merge),
        ))(i)
    }
}

/// `posting_cost = "{" cost_component? ("," cost_component)* "}" | "{{" ... "}}"`: the cost spec
/// of a posting, read with `date` as the date grammar of the format. Its components come in any
/// order, as in beancount: `{150 USD}` per unit, `{{1500 USD}}` in total, `{}` for whatever lot
/// booking finds, `{150 USD, 2024-01-15, "lot"}` with the acquisition date and the label of the
/// lot, `{2024-01-15}` or `{"lot"}` alone (also written `{, "lot"}`), the compound cost
/// `{100 # 5 USD}` of a per-unit and a total part, and `{*}`, beancount's merge-cost marker.
///
/// A later date or label replaces an earlier one, as it always did. A second cost number
/// (`{100 USD, 200 USD}`) does not read: beancount reports it and keeps the first, and silently
/// taking either would change what the lot costs. Nor does a compound cost in total braces
/// (`{{100 # 5 USD}}`), which beancount reads with an error, dropping the per-unit part.
pub fn posting_cost_with_date(i: &str, date: fn(&str) -> IResult<&str, Date>) -> IResult<&str, PostingCost> {
    // `{{ }}` is a total cost, `{ }` is a per-unit cost.
    let (i, total) = alt((value(true, tag("{{")), value(false, char('{'))))(i)?;
    let (i, _) = space0(i)?;
    let (i, first) = opt(cost_component(date))(i)?;
    let (i, rest) = many0(preceded(tuple((space0, char(','), space0)), cost_component(date)))(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = if total { value((), tag("}}"))(i)? } else { value((), char('}'))(i)? };

    let mut cost = PostingCost {
        total,
        ..PostingCost::default()
    };
    let unread = || nom::Err::Error(nom::error::Error::new(i, nom::error::ErrorKind::Verify));
    for component in first.into_iter().chain(rest) {
        match component {
            CostComponent::Amount(_) | CostComponent::Compound(..) if cost.base.is_some() => return Err(unread()),
            CostComponent::Amount(amount) => cost.base = Some(amount),
            CostComponent::Compound(per_unit, total) => {
                cost.base = Some(per_unit);
                cost.compound_total = Some(total);
            }
            CostComponent::Date(d) => cost.date = Some(d),
            CostComponent::Label(l) => cost.label = Some(l),
            CostComponent::Merge => cost.merge = true,
        }
    }
    if total && cost.compound_total.is_some() {
        return Err(unread());
    }
    Ok((i, cost))
}

/// The cost spec of a posting in a zhang file ([`posting_cost_with_date`] with zhang's dates).
pub fn posting_cost(i: &str) -> IResult<&str, PostingCost> {
    posting_cost_with_date(i, parse_date)
}

/// `posting_price = "@@" ... | "@" ...`
pub fn posting_price(i: &str) -> IResult<&str, SingleTotalPrice> {
    alt((
        map(preceded(pair(tag("@@"), space0), posting_amount), SingleTotalPrice::Total),
        map(preceded(pair(char('@'), space0), posting_amount), SingleTotalPrice::Single),
    ))(i)
}

pub type PostingMeta = (Option<PostingCost>, Option<SingleTotalPrice>);

/// `posting_meta = ("{" ... "}")? space* posting_price?`
fn posting_meta(i: &str) -> IResult<&str, PostingMeta> {
    let (i, cost) = opt(preceded(space0, posting_cost))(i)?;
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

/// Whether `c` is a flag of its own: `*`, `!`, `#`, `&`, `?`, `%` or an uppercase ASCII letter, the
/// characters beancount 3 reads as the flag of a transaction or of a posting.
pub fn is_flag_char(c: char) -> bool {
    matches!(c, '*' | '!' | '#' | '&' | '?' | '%') || c.is_ascii_uppercase()
}

/// Whether `c` is the flag of a posting in a zhang file: a [flag character](is_flag_char) other
/// than `#`. An indented line starting with `#` is a comment in zhang, such as a posting commented
/// out with `# Assets:Cash -10 CNY`, and stays one: a comment never becomes a posting. `*` is a
/// posting flag, as in beancount: `* Assets:Cash -10 CNY` is a posting flagged `*`.
pub fn is_posting_flag_char(c: char) -> bool {
    is_flag_char(c) && c != '#'
}

/// `flag_char = "*" | "!" | "#" | "&" | "?" | "%" | ASCII_ALPHA_UPPER`
pub fn flag_char(i: &str) -> IResult<&str, Flag> {
    map(satisfy(is_flag_char), |c| Flag::from_str(&c.to_string()).expect("invalid flag"))(i)
}

/// `flag = "txn" | flag_char`
fn flag(i: &str) -> IResult<&str, Flag> {
    alt((
        // beancount's `txn` keyword is the explicit form of a completed transaction
        map(tag("txn"), |_| Flag::Okay),
        flag_char,
    ))(i)
}

/// `transaction_flag = space+ flag`
pub fn transaction_flag(i: &str) -> IResult<&str, Flag> {
    preceded(space1, flag)(i)
}

/// `posting_flag = ("*" | "!" | "&" | "?" | "%" | ASCII_ALPHA_UPPER) space+`: the flag of a
/// posting, before its account, such as the `!` of `! Assets:Cash -10 CNY`. Beancount takes no `txn`
/// there, and `#` starts a comment (see [`is_posting_flag_char`]). The space is required.
fn posting_flag(i: &str) -> IResult<&str, Flag> {
    terminated(
        map(satisfy(is_posting_flag_char), |c| Flag::from_str(&c.to_string()).expect("invalid flag")),
        space1,
    )(i)
}

/// `transaction_posting = posting_flag? account_name (space+ posting_unit)?`
fn transaction_posting(i: &str) -> IResult<&str, Posting> {
    let (i, flag) = opt(posting_flag)(i)?;
    let (i, account) = account_name(i)?;
    let (i, unit) = opt(preceded(space1, posting_unit))(i)?;

    let mut posting = Posting {
        flag,
        account,
        units: None,
        cost: None,
        price: None,
        comment: None,
        meta: Meta::default(),
        written: None,
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

/// One indented line inside a transaction.
pub enum TransactionLine {
    /// boxed: a posting is far larger than the other lines
    Posting(Box<Posting>),
    Meta((String, ZhangString)),
    /// a comment or a whitespace-only line
    Other,
}

/// The width in columns of the leading whitespace `indent` of a line; a tab advances to
/// the next multiple of four columns.
pub fn indentation_width(indent: &str) -> usize {
    indent.chars().fold(0, |width, c| if c == '\t' { (width / 4 + 1) * 4 } else { width + 1 })
}

/// A single indented line inside a transaction: a posting, a metadata pair, or an
/// (ignored) comment / blank line, with the width of its indentation.
fn transaction_line(i: &str) -> IResult<&str, (usize, TransactionLine)> {
    let (i, _) = line_ending(i)?;
    let (i, indent) = space1(i)?;
    let (i, content) = opt(alt((
        map(transaction_posting, |posting| TransactionLine::Posting(Box::new(posting))),
        map(key_value_line, TransactionLine::Meta),
    )))(i)?;
    let (i, _) = space0(i)?;
    let (i, comment) = opt(valuable_comment_body)(i)?;

    let line = match (content, comment) {
        (Some(TransactionLine::Posting(posting)), Some(comment)) => TransactionLine::Posting(Box::new(posting.set_comment(comment))),
        (Some(line), _) => line,
        (None, _) => TransactionLine::Other,
    };
    Ok((i, (indentation_width(indent), line)))
}

/// `transaction_lines = transaction_line+`
fn transaction_lines(i: &str) -> IResult<&str, Vec<(usize, TransactionLine)>> {
    many1(transaction_line)(i)
}

/// A tag (`#name`) or link (`^name`) preceded by optional whitespace. The bool is
/// `true` for a tag, `false` for a link.
pub fn spaced_tag_or_link(i: &str) -> IResult<&str, (bool, String)> {
    preceded(
        space0,
        alt((
            map(preceded(char('#'), unquote_string_raw), |s: &str| (true, s.to_string())),
            map(preceded(char('^'), unquote_string_raw), |s: &str| (false, s.to_string())),
        )),
    )(i)
}

/// `tags_or_links = (space* (tag | link))*`
pub fn tags_or_links(i: &str) -> IResult<&str, (Vec<String>, Vec<String>)> {
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

/// The tags and links after the string of a note or document, as the AST keeps
/// them: `None` when there are none.
type TagAndLinkSets = (Option<HashSet<String>>, Option<HashSet<String>>);

pub fn tag_and_link_sets(i: &str) -> IResult<&str, TagAndLinkSets> {
    let (i, (tags, links)) = tags_or_links(i)?;
    let set = |items: Vec<String>| (!items.is_empty()).then(|| items.into_iter().collect::<HashSet<_>>());
    Ok((i, (set(tags), set(links))))
}

// ---------------------------------------------------------------------------
// metadata
// ---------------------------------------------------------------------------

/// An unquoted metadata key: a bare word that does not start with a comment prefix, so an
/// indented line such as `;path: "C:\x"` stays a comment, nor with `*`. The exporter writes any
/// other key quoted, for both formats, and `*` starts a comment in a beancount file: a key this
/// grammar reads bare reads back as a key in either format. In a zhang file `*path: "x"` is then
/// neither metadata nor a comment, and an error.
pub fn meta_key(i: &str) -> IResult<&str, &str> {
    verify(unquote_string_raw, |key: &str| comment_prefix(key).is_err() && !key.starts_with('*'))(i)
}

/// Where a bare metadata value may end: the end of the input or of the line, whitespace, or a
/// `;` comment.
fn bare_value_end(i: &str) -> IResult<&str, ()> {
    alt((value((), eof), value((), one_of(" \t\r\n;"))))(i)
}

/// `bare_meta_value`: a metadata value written without quotes, kept as it is written. A bare
/// word, such as `USD`, `TRUE`, `NULL`, `1.5` or `2024-01-15`, and the values beancount writes
/// bare that a word does not cover: an account (`Assets:Bank`), an amount (`10 USD`) and a
/// number with group separators or arithmetic (`1,000`, `(1 + 2) * 3`). An account, amount or
/// number is only read where the value ends there ([`bare_value_end`]), so `1.0.0` is the word
/// `1.0.0` and not the number `1.0` followed by `.0`. A value never starts with whitespace,
/// which a number expression would otherwise skip: ` 10` is not a bare value that reads back.
pub fn bare_meta_value(i: &str) -> IResult<&str, &str> {
    let (i, _) = peek(satisfy(|c: char| !c.is_whitespace()))(i)?;
    alt((
        terminated(
            recognize(alt((value((), account_name), value((), posting_amount), value((), number_expr)))),
            peek(bare_value_end),
        ),
        unquote_string_raw,
    ))(i)
}

/// `meta_value = quote_string | bare_meta_value`
pub fn meta_value(i: &str) -> IResult<&str, ZhangString> {
    alt((quote_string, map(bare_meta_value, |s: &str| ZhangString::UnquoteString(s.to_string()))))(i)
}

/// `key_value_line = (meta_key | quote_string) space* ":" space* meta_value`
pub fn key_value_line(i: &str) -> IResult<&str, (String, ZhangString)> {
    let (i, key) = alt((map(meta_key, str::to_owned), map(quote_string, |key| key.to_plain_string())))(i)?;
    let (i, _) = space0(i)?;
    let (i, _) = char(':')(i)?;
    let (i, _) = space0(i)?;
    let (i, value) = meta_value(i)?;
    Ok((i, (key, value)))
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
pub fn metas_block(i: &str) -> IResult<&str, Meta> {
    map(many1(meta_line), |pairs| pairs.into_iter().collect())(i)
}

// ---------------------------------------------------------------------------
// directive bodies (everything after `date keyword`)
// ---------------------------------------------------------------------------

pub fn comma_separator(i: &str) -> IResult<&str, ()> {
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
    let (i, (tags, links)) = tag_and_link_sets(i)?;
    Ok((
        i,
        Directive::Note(Note {
            date,
            account,
            comment,
            tags,
            links,
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

/// `pad = date "pad" account account`, as in beancount
fn pad_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, pad) = account_name(i)?;
    Ok((
        i,
        Directive::Pad(Pad {
            date,
            account,
            pad,
            meta: Meta::default(),
        }),
    ))
}

fn document_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, account) = account_name(i)?;
    let (i, _) = space1(i)?;
    let (i, filename) = string(i)?;
    let (i, (tags, links)) = tag_and_link_sets(i)?;
    Ok((
        i,
        Directive::Document(Document {
            date,
            account,
            filename,
            tags,
            links,
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

/// `query = date "query" space+ string space+ quote_string`; the query text is
/// kept verbatim and not validated here.
fn query_body(date: Date, i: &str) -> IResult<&str, Directive> {
    let (i, _) = space1(i)?;
    let (i, name) = string(i)?;
    let (i, _) = space1(i)?;
    let (i, query_string) = quote_string(i)?;
    Ok((
        i,
        Directive::Query(Query {
            date,
            name,
            query_string,
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

pub fn string_or_account(i: &str) -> IResult<&str, StringOrAccount> {
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
        "pad" => pad_body(date, rest),
        "document" => document_body(date, rest),
        "price" => price_body(date, rest),
        "event" => event_body(date, rest),
        "query" => query_body(date, rest),
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
    // A metadata line belongs to the posting before it only when it is indented deeper
    // than that posting's line, and to the transaction otherwise, wherever it is: zhang
    // wrote transaction metadata after the postings, at their indentation, until #457.
    let mut posting_indent = None;
    for (indent, line) in lines {
        match line {
            TransactionLine::Posting(posting) => {
                transaction.postings.push(*posting);
                posting_indent = Some(indent);
            }
            TransactionLine::Meta((key, value)) => match (posting_indent, transaction.postings.last_mut()) {
                (Some(posting_indent), Some(posting)) if indent > posting_indent => posting.meta.insert(key, value),
                _ => transaction.meta.insert(key, value),
            },
            TransactionLine::Other => {}
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

// ---------------------------------------------------------------------------
// names written unquoted
// ---------------------------------------------------------------------------
//
// The exporter writes account names, commodities, tags, links, metadata keys and
// flags as they are, without quotes. These checks run the grammar above on such a
// name, so a caller that builds directives from user input (the server) can reject
// a name that would not read back. The beancount parser accepts exactly the same
// names, which its tests check.

/// Whether `parser` reads the whole of `text`.
fn reads_all<'a, O>(mut parser: impl FnMut(&'a str) -> IResult<&'a str, O>, text: &'a str) -> bool {
    matches!(parser(text), Ok(("", _)))
}

/// `text` read whole as the cost spec of a posting ([`posting_cost`]): `{150 USD}`, `{{1500 USD}}`,
/// `{}`, `{150 USD, 2024-01-15, "lot"}` or any other form it reads. `None` when it is not one,
/// such as `150 USD` without the braces or a cost followed by anything else: a caller that takes
/// a cost as text (the server's transaction edit) writes exactly what the ledger reads back, or
/// nothing.
pub fn read_posting_cost(text: &str) -> Option<PostingCost> {
    match posting_cost(text) {
        Ok(("", cost)) => Some(cost),
        _ => None,
    }
}

/// `text` read whole as the units of a posting ([`posting_amount`]): a number, which may be an expression such as
/// `(10 + 2) / 4` and may group its digits with `,` or `_`, then the commodity, as in `-1,000.50 CNY`. `None` when it
/// is not one, such as a number without a commodity or units followed by a cost or a price.
pub fn read_posting_amount(text: &str) -> Option<Amount> {
    match posting_amount(text) {
        Ok(("", amount)) => Some(amount),
        _ => None,
    }
}

/// `text` read whole as a number ([`number_expr`]): `-1,000.50`, or an expression such as `(10 + 2) / 4`. `None` when
/// it is not one.
pub fn read_number(text: &str) -> Option<BigDecimal> {
    match number_expr(text) {
        Ok(("", number)) => Some(number),
        _ => None,
    }
}

/// `text` read whole as the price annotation of a posting ([`posting_price`]): `@ 6 USD` per unit or
/// `@@ 60 USD` in total. `None` when it is not one, such as `6 USD` without the `@`.
pub fn read_posting_price(text: &str) -> Option<SingleTotalPrice> {
    match posting_price(text) {
        Ok(("", price)) => Some(price),
        _ => None,
    }
}

/// Whether `name` is an account name, such as `Assets:Bank:Checking`, that reads
/// back unchanged.
pub fn is_valid_account_name(name: &str) -> bool {
    reads_all(account_name, name)
}

/// Whether `name` is a commodity name, such as `CNY` or `VBMPX`, that reads back
/// unchanged.
pub fn is_valid_commodity_name(name: &str) -> bool {
    reads_all(commodity_name, name)
}

/// Whether `name` is a tag (`#name`) or link (`^name`) that reads back unchanged.
pub fn is_valid_tag_or_link(name: &str) -> bool {
    reads_all(unquote_string_raw, name)
}

/// Whether `key` is a metadata key that reads back unchanged when written unquoted.
pub fn is_valid_meta_key(key: &str) -> bool {
    reads_all(meta_key, key)
}

/// Whether `value` is a metadata value that reads back unchanged when written unquoted:
/// the value grammar ([`meta_value`]) reads all of it as one bare value, such as `1.5`,
/// `2024-01-15`, `TRUE`, `Assets:Bank` or `10 USD`, and not as a quoted string or as a
/// shorter value followed by something else.
pub fn is_valid_bare_meta_value(value: &str) -> bool {
    matches!(meta_value(value), Ok(("", ZhangString::UnquoteString(read))) if read == value)
}

/// Whether `flag` is a transaction flag that reads back as a flag.
pub fn is_valid_transaction_flag(flag: &str) -> bool {
    reads_all(self::flag, flag)
}

/// The length of the header line of `text`, a transaction as written in zhang or
/// beancount syntax (the beancount parser reads the same header): everything before the
/// line ending of its first line. That is not the first line ending of `text` when a
/// quoted payee or narration spans several lines. `None` when `text` does not start with
/// a transaction header followed by a line ending, such as a one-line `balance`.
pub fn transaction_header_len(text: &str) -> Option<usize> {
    let header = tuple((
        parse_date,
        opt(transaction_flag),
        many_m_n(0, 2, preceded(space1, quote_string)),
        tags_or_links,
        space0,
        opt(inline_comment),
    ));
    let (rest, _) = terminated(header, peek(line_ending))(text).ok()?;
    Some(offset(text, rest))
}

/// Whether `text`, a transaction as written in zhang or beancount syntax, has text the exporter does not write back
/// when the transaction is rewritten from its directive, as an edit through the API does: a comment on its header
/// line, an indented line that is neither a posting nor a metadata line and not blank, such as a comment line
/// between postings, or a comment after a metadata line. The comment of a posting line is the posting's, and is
/// written back. The journal reports this (#473), so that a client warns before an edit drops such text.
///
/// The beancount parser reads the same lines, except that a posting flagged `#` (`# Assets:Cash 10 CNY`), which
/// only beancount reads as a posting, counts as a comment line here: a warning too many, never one too few.
pub fn transaction_has_unexported_text(text: &str) -> bool {
    let Some(header_len) = transaction_header_len(text) else {
        return false;
    };
    // the header without its comment; what the comment-less grammar leaves of the header is the comment
    let mut structured = tuple((
        parse_date,
        opt(transaction_flag),
        many_m_n(0, 2, preceded(space1, quote_string)),
        tags_or_links,
        space0,
    ));
    if let Ok((rest, _)) = structured(&text[..header_len]) {
        if !rest.is_empty() {
            return true;
        }
    }
    text[header_len..]
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.trim().is_empty())
        .any(|line| !transaction_line_is_written_back(line))
}

/// Whether `line`, an indented line of a transaction, is written back by the exporter: a posting line, with or
/// without its comment, or a metadata line without one.
fn transaction_line_is_written_back(line: &str) -> bool {
    let mut content = preceded(space1, alt((map(transaction_posting, |_| true), map(key_value_line, |_| false))));
    let Ok((rest, is_posting)) = content(line) else {
        return false;
    };
    let rest = rest.trim_start_matches([' ', '\t']);
    rest.is_empty() || (is_posting && matches!(valuable_comment_body(rest), Ok(("", _))))
}

fn error_at(original: &str, rest: &str, message: &str) -> ParseError {
    let (line, column) = line_column(original, (1, 0), offset(original, rest));
    ParseError {
        message: format!("failed to parse zhang file: {} at line {}, column {}", message, line, column),
    }
}

/// The 1-based line and column of byte `offset` in `text`, the column counting characters. `from` is the
/// `(line, byte offset)` of a position already known, at or before `offset`, so a caller walking a file counts
/// every line once; `(1, 0)` is the start of the text.
pub fn line_column(text: &str, from: (usize, usize), offset: usize) -> (usize, usize) {
    let line = from.0 + text[from.1..offset].bytes().filter(|byte| *byte == b'\n').count();
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    (line, text[line_start..offset].chars().count() + 1)
}

/// Parse a full zhang text file into a list of spanned directives.
pub fn parse(input_str: &str, file: impl Into<Option<PathBuf>>) -> Result<Vec<Spanned<Directive>>, ParseError> {
    parse_items(input_str, file.into(), content_item, error_at)
}

/// Read a whole text file with a format's item parser and error constructor. Both formats
/// share whitespace handling, byte spans, escape-error locations and the progress guard;
/// their directive grammars and public error types remain with the callers.
pub fn parse_items<'a, T: std::fmt::Debug + PartialEq, E>(
    input_str: &'a str, file: Option<PathBuf>, mut item: impl FnMut(&'a str) -> IResult<&'a str, Option<T>>, error_at: impl Fn(&'a str, &'a str, &str) -> E,
) -> Result<Vec<Spanned<T>>, E> {
    // a leading byte order mark (#505) is no part of the text: the spans and the error positions count from the
    // text after it, which is what the write paths edit (`crate::data_source::FileText`)
    let original = input_str.strip_prefix(BOM).unwrap_or(input_str);
    let mut rest = original;
    let mut directives = Vec::new();
    // (line, byte offset) where the last directive starts: the next one's line is counted from there
    let mut position = (1, 0);

    loop {
        while let Ok((next, _)) = blank_line(rest) {
            rest = next;
        }
        if rest.is_empty() || rest.bytes().all(|byte| byte == b' ' || byte == b'\t') {
            break;
        }

        let start = offset(original, rest);
        let (next, directive) = item(rest).map_err(|err| match invalid_escape_at(&err) {
            Some(escape) => error_at(original, escape, "invalid escape sequence"),
            None => error_at(original, rest, "unexpected input"),
        })?;

        // Defensive: every successful item must make progress.
        if offset(original, next) == start {
            return Err(error_at(original, rest, "parser made no progress"));
        }

        if let Some(directive) = directive {
            let end = offset(original, next);
            let (line, column) = line_column(original, position, start);
            position = (line, start);
            directives.push(Spanned {
                data: directive,
                span: SpanInfo {
                    start,
                    end,
                    content: original[start..end].to_string(),
                    filename: file.clone(),
                    line: Some(line),
                    column: Some(column),
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

        #[test]
        fn should_parse_pad() {
            let pad = parse("2101-10-10 pad Assets:Hello Equity:Opening-Balances\n  note: \"opening\"", None)
                .unwrap()
                .remove(0);
            let mut meta = Meta::default();
            meta.insert("note".to_owned(), ZhangString::quote("opening"));
            assert_eq!(
                Directive::Pad(Pad {
                    date: Date::Date(NaiveDate::from_ymd_opt(2101, 10, 10).unwrap()),
                    account: Account::from_str("Assets:Hello").unwrap(),
                    pad: Account::from_str("Equity:Opening-Balances").unwrap(),
                    meta,
                }),
                pad.data
            );
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
    mod note_and_document_tags {
        use std::collections::HashSet;

        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        fn set(items: &[&str]) -> Option<HashSet<String>> {
            Some(items.iter().map(|it| (*it).to_owned()).collect())
        }

        #[test]
        fn notes_and_documents_keep_their_tags_and_links() {
            let directives = parse(
                "2020-01-10 note Assets:Bank \"x\" #t1 ^ln #t2 ; a comment\n\
                 2020-01-11 document Assets:Bank \"a.pdf\" ^l1 ^l2\n\
                 2020-01-12 note Assets:Bank \"y\" # a comment, not a tag\n\
                 2020-01-13 document Assets:Bank \"b.pdf\"\n  k: \"v\"\n",
                None,
            )
            .unwrap()
            .into_iter()
            .map(|it| it.data)
            .collect::<Vec<_>>();
            let Directive::Note(note) = &directives[0] else {
                panic!("{:?}", directives[0])
            };
            assert_eq!((note.tags.clone(), note.links.clone()), (set(&["t1", "t2"]), set(&["ln"])));
            let Directive::Document(document) = &directives[1] else {
                panic!("{:?}", directives[1])
            };
            assert_eq!((document.tags.clone(), document.links.clone()), (None, set(&["l1", "l2"])));
            let Directive::Note(note) = &directives[2] else {
                panic!("{:?}", directives[2])
            };
            assert_eq!((note.tags.clone(), note.links.clone()), (None, None));
            let Directive::Document(document) = &directives[3] else {
                panic!("{:?}", directives[3])
            };
            assert_eq!((document.tags.clone(), document.links.clone()), (None, None));
            assert_eq!(document.meta.get_one("k").map(|it| it.as_str()), Some("v"));
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
                assert_eq!(inner.amount.commodity, "CNY");
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
    mod query {

        use indoc::indoc;
        use zhang_ast::{Directive, Meta, ZhangString};

        use crate::data_type::text::parser::parse;

        #[test]
        fn should_parse() {
            let mut vec = parse(
                indoc! {r#"
                            2024-01-01 query "france-balances" "SELECT account, sum(position) WHERE 'trip-france' IN tags"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            let Directive::Query(inner) = directive else {
                panic!("expected a query directive, got {:?}", directive);
            };
            assert_eq!(inner.date, date!(2024, 1, 1));
            assert_eq!(inner.name, quote!("france-balances"));
            assert_eq!(inner.query_string, quote!("SELECT account, sum(position) WHERE 'trip-france' IN tags"));
            assert_eq!(inner.meta, Meta::default());
        }

        #[test]
        fn should_parse_with_meta_and_trailing_comment() {
            let mut vec = parse(
                indoc! {r#"
                            2024-01-01 query monthly "SELECT year, month, sum(position)" ; saved
                              owner: "alice"
                              category: "reports"
                        "#},
                None,
            )
            .unwrap();
            assert_eq!(vec.len(), 1);
            let directive = vec.pop().unwrap().data;
            let Directive::Query(inner) = directive else {
                panic!("expected a query directive, got {:?}", directive);
            };
            assert_eq!(inner.name, ZhangString::unquote("monthly"));
            assert_eq!(inner.query_string, quote!("SELECT year, month, sum(position)"));
            assert_eq!(inner.meta.get_one("owner"), Some(&quote!("alice")));
            assert_eq!(inner.meta.get_one("category"), Some(&quote!("reports")));
        }

        #[test]
        fn should_keep_multi_line_and_escaped_query_text() {
            let mut vec = parse("2024-01-01 query \"q\" \"SELECT payee\n  WHERE narration ~ \\\"x\\\"\"\n", None).unwrap();
            let Directive::Query(inner) = vec.pop().unwrap().data else {
                panic!("expected a query directive");
            };
            assert_eq!(inner.query_string, quote!("SELECT payee\n  WHERE narration ~ \"x\""));
        }

        #[test]
        fn should_reject_an_unquoted_query_text() {
            assert!(parse("2024-01-01 query name SELECT\n", None).is_err());
            assert!(parse("2024-01-01 query \"name\"\n", None).is_err());
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
        #[test]
        fn should_support_every_beancount_flag() {
            // beancount 3.2.3 reads `&`, `?` and `%` as flags too
            for flag in ["&", "?", "%"] {
                let trx = get_txn(&format!("2022-06-02 {flag} \"x\"\n  Assets:Card -1 USD\n  Expenses:Food\n"));
                assert_eq!(trx.flag, Some(Flag::Custom(flag.to_string())));
            }
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
            /// #497: the components of a cost spec come in any order, and each may stand alone, as in
            /// beancount: a date, a label (with or without the leading comma zhang took before), the
            /// compound cost `P # T CUR` and the merge-cost marker `*`
            #[test]
            fn should_read_every_cost_spec_form() {
                let usd = |number: &str| Amount::new(BigDecimal::from_str(number).unwrap(), "USD");
                let date = Date::Date(NaiveDate::from_ymd_opt(2024, 1, 10).unwrap());
                let cases = [
                    (
                        "{2024-01-10}",
                        PostingCost {
                            date: Some(date.clone()),
                            ..Default::default()
                        },
                    ),
                    (
                        "{\"b\"}",
                        PostingCost {
                            label: Some("b".to_owned()),
                            ..Default::default()
                        },
                    ),
                    (
                        "{, \"b\"}",
                        PostingCost {
                            label: Some("b".to_owned()),
                            ..Default::default()
                        },
                    ),
                    (
                        "{{\"b\"}}",
                        PostingCost {
                            label: Some("b".to_owned()),
                            total: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "{100 # 5 USD}",
                        PostingCost {
                            base: Some(usd("100")),
                            compound_total: Some(BigDecimal::from(5)),
                            ..Default::default()
                        },
                    ),
                    (
                        "{100#5.25 USD, 2024-01-10, \"a\"}",
                        PostingCost {
                            base: Some(usd("100")),
                            compound_total: Some(BigDecimal::from_str("5.25").unwrap()),
                            date: Some(date.clone()),
                            label: Some("a".to_owned()),
                            ..Default::default()
                        },
                    ),
                    (
                        "{*}",
                        PostingCost {
                            merge: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "{ * , \"b\" }",
                        PostingCost {
                            label: Some("b".to_owned()),
                            merge: true,
                            ..Default::default()
                        },
                    ),
                    (
                        "{\"a\", 2024-01-10, 100 USD}",
                        PostingCost {
                            base: Some(usd("100")),
                            date: Some(date.clone()),
                            label: Some("a".to_owned()),
                            ..Default::default()
                        },
                    ),
                    (
                        "{2024-01-10, 100 USD}",
                        PostingCost {
                            base: Some(usd("100")),
                            date: Some(date.clone()),
                            ..Default::default()
                        },
                    ),
                ];
                for (spec, expected) in cases {
                    let mut trx = get_first_posting(&format!("2024-02-01 \"sell\"\n  Assets:Broker -1 HOOL {spec}\n  Assets:Cash 120 USD\n"));
                    let posting = trx.postings.remove(0);
                    assert_eq!(posting.units, Some(Amount::new(BigDecimal::from(-1), "HOOL")), "{spec}");
                    assert_eq!(posting.cost, Some(expected), "{spec}");
                    assert_eq!(posting.price, None, "{spec}");
                }
            }

            /// a compound cost is not a total cost: `{{100 # 5 USD}}` does not read (beancount reads it with an
            /// error and drops the per-unit part), nor does a second cost number
            #[test]
            fn should_reject_a_compound_total_cost_and_a_second_cost_number() {
                for cost in ["{{100 # 5 USD}}", "{100 USD, 101 USD}"] {
                    let text = format!("2024-02-01 \"buy\"\n  Assets:Broker 10 HOOL {cost}\n  Assets:Cash -1005 USD\n");
                    assert!(parse(&text, None).is_err(), "{cost}");
                }
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

        #[test]
        fn header_len_ends_at_the_first_line_ending_outside_strings() {
            use crate::data_type::text::parser::transaction_header_len;

            let postings = "\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n";
            for header in [
                "2024-01-15 * \"Bob\" \"coffee\"",
                "2024-01-15 10:30:00 txn \"Bob\" \"coffee\" #trip ^inv-1",
                "2024-01-15 \"coffee\"",
                "2024-01-15 * \"Bob\" \"multi\nline narration\"",
                "2024-01-15 * \"Bob\n  Assets:Cash -1 CNY\" \"coffee\"",
                "2024-01-15 * \"Bob\" \"say \\\"hi\\\"\nthere\"",
                "2024-01-15 * \"Bob\" \"ends with a backslash \\\\\"",
                "2024-01-15 * \"Bob\" \"coffee\" ; a 5\" screen, \"quoted",
                "2024-01-15 * \"Bob\" \"coffee\" #trip // a comment",
            ] {
                let text = format!("{header}{postings}");
                assert_eq!(transaction_header_len(&text), Some(header.len()), "{text:?}");
                let crlf = format!("{header}\r\n  Assets:Cash -5 CNY\r\n");
                assert_eq!(transaction_header_len(&crlf), Some(header.len()), "{crlf:?}");
            }
            assert_eq!(transaction_header_len("2024-01-15 balance Assets:Cash 5 CNY"), None);
            assert_eq!(transaction_header_len("2024-01-15 balance Assets:Cash 5 CNY\n"), None);
        }

        /// A metadata line belongs to the posting before it only when it is indented
        /// deeper than the posting line; otherwise it is the transaction's.
        mod posting_metadata {
            use zhang_ast::{Meta, Transaction, ZhangString};

            use crate::data_type::text::parser::test::get_txn;

            /// The metadata as sorted `key=value` pairs.
            fn pairs(meta: &Meta) -> Vec<String> {
                let mut pairs = meta
                    .clone()
                    .get_flatten()
                    .into_iter()
                    .map(|(k, v)| format!("{}={}", k, v.as_str()))
                    .collect::<Vec<_>>();
                pairs.sort();
                pairs
            }

            fn posting_pairs(txn: &Transaction) -> Vec<Vec<String>> {
                txn.postings.iter().map(|posting| pairs(&posting.meta)).collect()
            }

            #[test]
            fn metadata_before_the_postings_is_the_transactions() {
                let txn = get_txn("2024-01-02 * \"Cafe\"\n  memo: \"m\"\n    deep: \"d\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n");
                assert_eq!(pairs(&txn.meta), vec!["deep=d", "memo=m"]);
                assert_eq!(posting_pairs(&txn), vec![Vec::<String>::new(), vec![]]);
            }

            #[test]
            fn metadata_indented_deeper_than_its_posting_is_the_postings() {
                let txn = get_txn(
                    "2024-01-02 * \"Cafe\"\n  Assets:Cash -5 CNY\n    receipt: \"r1\"\n    \"my key\": \"v\"\n  Expenses:Food 5 CNY\n   category: \"lunch\"\n",
                );
                assert!(pairs(&txn.meta).is_empty());
                assert_eq!(posting_pairs(&txn), vec![vec!["my key=v", "receipt=r1"], vec!["category=lunch"]]);
                assert_eq!(txn.postings[1].meta.get_one("category"), Some(&ZhangString::quote("lunch")));
            }

            #[test]
            fn metadata_at_or_above_the_posting_indentation_is_the_transactions_wherever_it_is() {
                // older zhang wrote transaction metadata after the postings, at their indentation
                let txn =
                    get_txn("2024-01-02 * \"Cafe\"\n    Assets:Cash -5 CNY\n    same: \"s\"\n  shallower: \"h\"\n    Expenses:Food 5 CNY\n    after: \"a\"\n");
                assert_eq!(pairs(&txn.meta), vec!["after=a", "same=s", "shallower=h"]);
                assert_eq!(posting_pairs(&txn), vec![Vec::<String>::new(), vec![]]);
            }

            #[test]
            fn metadata_before_between_and_after_the_postings() {
                let txn = get_txn(concat!(
                    "2024-01-02 * \"Shop\" \"mixed\"\n",
                    "  memo: \"before\"\n",
                    "  ; a comment\n",
                    "  Assets:Cash -10 CNY ; a posting comment\n",
                    "    ; a comment between a posting and its metadata\n",
                    "    receipt: \"r2\"\n",
                    "  between: \"b\"\n",
                    "      later: \"l\"\n",
                    "  Expenses:Food 6 CNY\n",
                    "  Expenses:Drinks 4 CNY\n",
                    "    rate: 1.5\n",
                    "    rate: 2\n",
                    "  after: \"a\"\n",
                ));
                assert_eq!(pairs(&txn.meta), vec!["after=a", "between=b", "memo=before"]);
                assert_eq!(posting_pairs(&txn), vec![vec!["later=l", "receipt=r2"], vec![], vec!["rate=1.5", "rate=2"]]);
                assert_eq!(txn.postings[0].comment.as_deref(), Some("a posting comment"));
                assert_eq!(txn.postings[2].meta.get_all("rate").len(), 2);
            }

            #[test]
            fn a_tab_indents_to_the_next_multiple_of_four_columns() {
                // a tab under a two-space posting is deeper, a tab under a four-space one is not
                let txn = get_txn("2024-01-02 * \"Cafe\"\n  Assets:Cash -5 CNY\n\tdeeper: \"d\"\n    Expenses:Food 5 CNY\n\tsame: \"s\"\n");
                assert_eq!(pairs(&txn.meta), vec!["same=s"]);
                assert_eq!(posting_pairs(&txn), vec![vec!["deeper=d"], vec![]]);

                let txn = get_txn("2024-01-02 * \"Cafe\"\n\tAssets:Cash -5 CNY\n\t\tdeeper: \"d\"\n\t  also: \"a\"\n    same: \"s\"\n");
                assert_eq!(pairs(&txn.meta), vec!["same=s"]);
                assert_eq!(posting_pairs(&txn), vec![vec!["also=a", "deeper=d"]]);
            }
        }

        /// A posting can carry its own flag before its account, as in beancount (#474), but not `*`
        /// or `#`, which start a comment in a zhang file.
        mod posting_flags {
            use std::str::FromStr;

            use bigdecimal::BigDecimal;
            use indoc::indoc;
            use zhang_ast::amount::Amount;
            use zhang_ast::{Directive, Flag, PostingCost, SingleTotalPrice, ZhangString};

            use crate::data_type::text::parser::parse;
            use crate::data_type::text::parser::test::get_txn;

            fn amount(number: &str, commodity: &str) -> Amount {
                Amount::new(BigDecimal::from_str(number).unwrap(), commodity)
            }

            #[test]
            fn a_posting_keeps_its_own_flag() {
                let txn = get_txn(indoc! {r#"
                    2024-01-10 * "Lunch"
                      ! Assets:Cash  -10 USD
                      & Expenses:Food 6 USD
                      Expenses:Drinks 4 USD
                "#});
                assert_eq!(txn.flag, Some(Flag::Okay));
                let flags = txn.postings.iter().map(|it| it.flag.clone()).collect::<Vec<_>>();
                assert_eq!(flags, vec![Some(Flag::Warning), Some(Flag::Custom("&".to_owned())), None]);
                let accounts = txn.postings.iter().map(|it| it.account.name()).collect::<Vec<_>>();
                assert_eq!(accounts, vec!["Assets:Cash", "Expenses:Food", "Expenses:Drinks"]);
                assert_eq!(txn.postings[0].units, Some(amount("-10", "USD")));
                assert_eq!(txn.postings[1].units, Some(amount("6", "USD")));
            }

            #[test]
            fn every_beancount_posting_flag_but_hash_is_read() {
                // the flags beancount 3.2.3 takes on a posting, but `#`; one or more spaces or tabs
                // follow it
                for (written, flag) in [
                    ("*", Flag::Okay),
                    ("!", Flag::Warning),
                    ("&", Flag::Custom("&".to_owned())),
                    ("?", Flag::Custom("?".to_owned())),
                    ("%", Flag::Custom("%".to_owned())),
                    ("P", Flag::BalancePad),
                    ("C", Flag::BalanceCheck),
                    ("X", Flag::Custom("X".to_owned())),
                ] {
                    for space in [" ", "   ", "\t"] {
                        let txn = get_txn(&format!("2024-01-10 * \"Lunch\"\n  {written}{space}Assets:Cash -10 USD\n  Expenses:Food\n"));
                        assert_eq!(txn.postings.len(), 2, "{written:?}");
                        assert_eq!(txn.postings[0].flag, Some(flag.clone()), "{written:?}");
                        assert_eq!(txn.postings[0].account.name(), "Assets:Cash", "{written:?}");
                        assert_eq!(txn.postings[1].flag, None, "{written:?}");
                    }
                }
            }

            #[test]
            fn a_flagged_posting_keeps_its_cost_price_comment_and_metadata() {
                let txn = get_txn(indoc! {r#"
                    2024-01-10 * "Broker" "sell"
                      ! Assets:Broker -5 AAPL {} @ 200 USD ; check the lot
                        receipt: "r-1"
                      ? Assets:Bank 1000 USD
                      Income:Gains
                "#});
                let sold = &txn.postings[0];
                assert_eq!(sold.flag, Some(Flag::Warning));
                assert_eq!(sold.units, Some(amount("-5", "AAPL")));
                assert_eq!(sold.cost, Some(PostingCost::default()));
                assert_eq!(sold.price, Some(SingleTotalPrice::Single(amount("200", "USD"))));
                assert_eq!(sold.comment.as_deref(), Some("check the lot"));
                assert_eq!(sold.meta.get_one("receipt"), Some(&ZhangString::quote("r-1")));
                assert_eq!(txn.postings[1].flag, Some(Flag::Custom("?".to_owned())));
                assert!(txn.postings[1].meta.clone().get_flatten().is_empty());
                assert!(txn.meta.clone().get_flatten().is_empty());
            }

            #[test]
            fn a_flagged_posting_can_leave_out_its_amount() {
                let txn = get_txn("2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  ! Expenses:Food\n");
                assert_eq!(txn.postings[1].flag, Some(Flag::Warning));
                assert_eq!(txn.postings[1].account.name(), "Expenses:Food");
                assert_eq!(txn.postings[1].units, None);

                let txn = get_txn("2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  ! Expenses:Food ; elided\n");
                assert_eq!(txn.postings[1].flag, Some(Flag::Warning));
                assert_eq!(txn.postings[1].units, None);
                assert_eq!(txn.postings[1].comment.as_deref(), Some("elided"));
            }

            #[test]
            fn a_flag_needs_a_space_before_the_account() {
                assert!(parse("2024-01-10 * \"Lunch\"\n  !Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
                assert!(parse("2024-01-10 * \"Lunch\"\n  ?Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
                assert!(parse("2024-01-10 * \"Lunch\"\n  *Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
                // `txn` is a transaction flag only, as in beancount
                assert!(parse("2024-01-10 * \"Lunch\"\n  txn Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
            }

            /// The text `without` with `line` indented and inserted as its line `at`.
            fn with_line_at(without: &str, line: &str, at: usize) -> String {
                let mut lines = without.lines().collect::<Vec<_>>();
                let indented = format!("  {line}");
                lines.insert(at, &indented);
                format!("{}\n", lines.join("\n"))
            }

            /// An indented line starting with `#` is a comment, as it always was in a zhang file,
            /// even when it reads like a flagged posting: a posting commented out that way must not
            /// come back and move a balance. The transaction is the same as without the line.
            #[test]
            fn indented_hash_lines_stay_comments() {
                let without = "2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  Expenses:Food\n";
                let expected = get_txn(without);
                for line in [
                    "# Assets:Cash -10 USD",
                    "#   Expenses:Food",
                    "#\tAssets:Broker -5 AAPL {} @ 200 USD ; note",
                    "#Assets:Cash -10 USD",
                    "# Assets",
                    "#",
                ] {
                    for at in [1, 2, 3] {
                        let text = with_line_at(without, line, at);
                        assert_eq!(get_txn(&text), expected, "{text:?}");
                    }
                }
                // and a whole ledger reads the same directives with the line or without it
                let ledger = "2024-01-01 open Assets:Cash\n2024-01-01 open Expenses:Food\n\n";
                let directives = |text: &str| parse(text, None).unwrap().into_iter().map(|it| it.data).collect::<Vec<Directive>>();
                let commented = format!("{ledger}2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  # Assets:Cash -99 USD\n  Expenses:Food\n");
                assert_eq!(directives(&commented), directives(&format!("{ledger}{without}")));
            }

            /// `*` does not start a comment in a zhang file: an indented `* Assets:Cash -5 USD` is a
            /// posting flagged `*`, as in beancount, and books. An indented line starting with `*`
            /// that is not a posting, such as a note or a flag without its space, is an error, not a
            /// comment: a line dropped in silence could be a posting meant to book.
            #[test]
            fn an_indented_star_line_is_a_posting_or_an_error() {
                let without = "2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  Expenses:Food\n";
                for (line, units) in [
                    ("* Assets:Cash -5 USD", Some(amount("-5", "USD"))),
                    ("*\tExpenses:Food 3 USD ; note", Some(amount("3", "USD"))),
                    ("* Assets:Broker -5 AAPL {} @ 200 USD", Some(amount("-5", "AAPL"))),
                    ("* Expenses:Drinks", None),
                ] {
                    for at in [1, 2, 3] {
                        let text = with_line_at(without, line, at);
                        let txn = get_txn(&text);
                        assert_eq!(txn.postings.len(), 3, "{text:?}");
                        assert_eq!(txn.postings[at - 1].flag, Some(Flag::Okay), "{text:?}");
                        assert_eq!(txn.postings[at - 1].units, units, "{text:?}");
                        assert_eq!(txn.postings.iter().filter(|it| it.flag.is_some()).count(), 1, "{text:?}");
                    }
                }
                let txn = get_txn("2024-01-10 * \"Lunch\"\n  Assets:Cash -10 USD\n  *\tExpenses:Food 3 USD ; note\n  Expenses:Drinks\n");
                assert_eq!(txn.postings[1].comment.as_deref(), Some("note"));
                let txn = get_txn("2024-01-10 * \"Broker\"\n  * Assets:Broker -5 AAPL {} @ 200 USD\n  Assets:Bank\n");
                assert_eq!(txn.postings[0].cost, Some(PostingCost::default()));
                assert_eq!(txn.postings[0].price, Some(SingleTotalPrice::Single(amount("200", "USD"))));

                for line in ["* a note", "*Assets:Cash -10 USD", "*Assets:Cash", "*path: \"x\"", "*", "** heading"] {
                    for at in [1, 2, 3] {
                        let text = with_line_at(without, line, at);
                        let error = parse(&text, None).expect_err(&text).to_string();
                        assert!(error.contains(&format!("unexpected input at line {}, column 3", at + 1)), "{text:?}: {error}");
                    }
                }
            }
        }
    }

    /// The comment prefixes of a zhang file, `;`, `#` and `//`, at the start of a line or after a
    /// directive. `*` is not one.
    mod comment {
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;

        #[test]
        fn semicolon_hash_and_slashes_start_a_comment() {
            for prefix in [";", "#", "//"] {
                let text = format!("{prefix} Options\n{prefix}{prefix} Banking\n2024-01-01 open Assets:Cash {prefix} opened\n");
                let directives = parse(&text, None).unwrap();
                assert_eq!(directives.len(), 3, "{prefix}");
                assert!(
                    matches!(&directives[0].data, Directive::Comment(comment) if comment.content == "Options"),
                    "{prefix}"
                );
                assert!(matches!(directives[1].data, Directive::Comment(_)), "{prefix}");
                assert!(matches!(directives[2].data, Directive::Open(_)), "{prefix}");
            }
        }

        /// A line starting with `*` at the start of a zhang file, such as an org-mode heading, is an
        /// error, not a comment: `*` is a flag, not a comment prefix. Such a file must use `;` or `#`.
        #[test]
        fn a_star_heading_is_a_parse_error() {
            for heading in ["* Options", "** Tax Year 2015", "*", "*Options"] {
                let text = format!("2024-01-01 open Assets:Cash\n\n{heading}\n\n2024-01-01 open Expenses:Food\n");
                let error = parse(&text, None).expect_err(&text).to_string();
                assert!(error.contains("unexpected input at line 3, column 1"), "{heading:?}: {error}");
            }
            // nor is `*` a comment after a directive or a metadata line
            assert!(parse("2024-01-01 open Assets:Cash * opened\n", None).is_err());
            assert!(parse("2024-01-01 open Assets:Cash\n  key: \"v\" * noted\n", None).is_err());
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

    /// String escaping (issue #442); the rules live in `crate::utils::string_`.
    mod escaping {
        use zhang_ast::Directive;

        use crate::data_type::text::parser::parse;
        use crate::data_type::text::parser::test::get_txn;
        use crate::utils::string_::test::{random_string, XorShift};

        fn narration(quoted: &str) -> String {
            let content = format!("2024-01-01 * {quoted}\n  Assets:Cash -5 CNY\n  Expenses:Food\n");
            get_txn(&content).narration.unwrap().to_plain_string()
        }

        fn query_text(content: &str) -> String {
            match parse(content, None).unwrap().pop().unwrap().data {
                Directive::Query(query) => query.query_string.to_plain_string(),
                other => panic!("expected a query directive, got {other:?}"),
            }
        }

        fn parse_error(content: &str) -> String {
            parse(content, None).expect_err(content).to_string()
        }

        #[test]
        fn should_keep_an_unknown_escape_in_a_query_verbatim() {
            let expected = r"SELECT narration WHERE narration ~ '\d+'";
            assert_eq!(query_text(r#"2014-01-01 query "x" "SELECT narration WHERE narration ~ '\d+'""#), expected);
            // the doubled backslash the docs used to require still reads the same
            assert_eq!(query_text(r#"2014-01-01 query "x" "SELECT narration WHERE narration ~ '\\d+'""#), expected);
        }

        #[test]
        fn should_read_strings_written_by_the_new_exporter() {
            assert_eq!(narration(r#""coffee $5""#), "coffee $5");
            assert_eq!(narration("\"SELECT\u{a0}account\u{2028}😀 你好\""), "SELECT\u{a0}account\u{2028}😀 你好");
            assert_eq!(narration(r#""a \"quote\" and a \\ backslash""#), r#"a "quote" and a \ backslash"#);
            assert_eq!(narration(r#""tab\there\u0007""#), "tab\there\u{07}");
        }

        #[test]
        fn should_read_escapes_written_by_older_versions() {
            assert_eq!(narration(r#""coffee \$5""#), "coffee $5");
            assert_eq!(narration(r#""run \`ls\`""#), "run `ls`");
            assert_eq!(narration(r#""a\u{a0}b""#), "a\u{a0}b");
            assert_eq!(narration(r#""smile \u{1F600}""#), "smile 😀");
            assert_eq!(narration(r#""line\u{2028}separator""#), "line\u{2028}separator");
            assert_eq!(narration(r#""\a\v\e\b\f""#), "\u{07}\u{0b}\u{1b}\u{08}\u{0c}");
            assert_eq!(
                query_text(r#"2014-01-01 query "q" "SELECT\u{a0}account""#),
                "SELECT\u{a0}account",
                "the separator escape from issue #442"
            );
        }

        #[test]
        fn should_report_a_malformed_escape_at_the_escape() {
            for escape in [r"\u{110000}", r"\u{D800}", r"\uZZZZ", r"\u{}", r"\uD800"] {
                let line = format!(r#"2024-01-01 note Assets:Cash "bad {escape} escape""#);
                let column = line.find('\\').unwrap() + 1;
                let error = parse_error(&format!("2024-01-01 open Assets:Cash\n{line}\n"));
                assert_eq!(
                    error,
                    format!("failed to parse zhang file: invalid escape sequence at line 2, column {column}"),
                    "{escape}"
                );
            }
        }

        #[test]
        fn should_reject_an_unterminated_string_without_panicking() {
            // a lone backslash before the closing quote escapes the quote, so the string never ends
            for content in [
                "2024-01-01 note Assets:Cash \"abc\\",
                "2024-01-01 note Assets:Cash \"abc\\\"\n",
                "2024-01-01 open Assets:Cash\n2024-01-01 note Assets:Cash \"abc\\\"\n2024-01-02 open Assets:Bank\n",
            ] {
                assert!(
                    parse_error(content).starts_with("failed to parse zhang file: unexpected input at line"),
                    "{content:?}"
                );
            }
        }

        #[test]
        fn a_ledger_with_an_unknown_escape_in_a_saved_query_loads() {
            use std::sync::Arc;

            use crate::data_source::LocalFileSystemDataSource;
            use crate::data_type::text::ZhangDataType;
            use crate::ledger::Ledger;

            let temp_dir = tempfile::tempdir().unwrap();
            std::fs::write(
                temp_dir.path().join("main.zhang"),
                "2014-01-01 query \"x\" \"SELECT narration WHERE narration ~ '\\d+'\"\n",
            )
            .unwrap();
            let source = LocalFileSystemDataSource::new(ZhangDataType {});
            let ledger = Ledger::load_with_data_source(temp_dir.path().to_path_buf(), "main.zhang".to_string(), Arc::new(source)).unwrap();
            let queries = ledger.operations().queries().unwrap();
            assert_eq!(queries.len(), 1);
            assert_eq!(queries[0].query, r"SELECT narration WHERE narration ~ '\d+'");
        }

        #[test]
        fn should_keep_indented_comment_like_lines_out_of_metadata() {
            for prefix in [";", "#", "//"] {
                let line = format!("  {prefix}path: \"C:\\Users\\me\"");
                let txn = get_txn(&format!("2024-01-01 * \"x\"\n  Assets:Cash -5 CNY\n{line}\n  Expenses:Food\n"));
                assert!(txn.meta.get_one(&format!("{prefix}path")).is_none(), "{line}");
                assert_eq!(txn.postings.len(), 2, "{line}");

                let directives = parse(&format!("2024-01-01 open Assets:Cash\n{line}\n"), None).unwrap();
                assert_eq!(directives.len(), 2, "{line}");
                let Directive::Open(open) = &directives[0].data else {
                    panic!("expected an open directive, got {:?}", directives[0].data);
                };
                assert!(open.meta.get_one(&format!("{prefix}path")).is_none(), "{line}");
                assert!(matches!(directives[1].data, Directive::Comment(_)), "{line}");
            }
            // `*` starts no comment, and no bare metadata key either: such a line is an error
            let line = "  *path: \"C:\\Users\\me\"";
            assert!(parse(&format!("2024-01-01 * \"x\"\n  Assets:Cash -5 CNY\n{line}\n  Expenses:Food\n"), None).is_err());
            assert!(parse(&format!("2024-01-01 open Assets:Cash\n{line}\n"), None).is_err());
        }

        #[test]
        fn should_still_read_plain_and_quoted_metadata_keys() {
            let directives = parse("2024-01-01 open Assets:Cash\n  path: \"a\"\n  \";path\": \"b\"\n  a;b: \"c\"\n", None).unwrap();
            assert_eq!(directives.len(), 1);
            let Directive::Open(open) = &directives[0].data else {
                panic!("expected an open directive, got {:?}", directives[0].data);
            };
            assert_eq!(open.meta.get_one("path").map(|it| it.as_str()), Some("a"));
            assert_eq!(open.meta.get_one(";path").map(|it| it.as_str()), Some("b"));
            assert_eq!(open.meta.get_one("a;b").map(|it| it.as_str()), Some("c"));
        }

        #[test]
        fn should_never_panic_on_random_string_bodies() {
            let mut rng = XorShift::new(0x0bad_5eed);
            for _ in 0..5000 {
                // random raw bodies, including stray quotes, backslashes and broken escapes
                let mut body = random_string(&mut rng);
                if rng.below(3) == 0 {
                    let escapes = [r"\u{", r"\u{D800}", r"\u{110000}", r"\u12", r"\uDBFF\u", r"\", r"\d", r"\u{1F600}"];
                    body.push_str(escapes[rng.below(escapes.len())]);
                }
                let content = format!("2024-01-01 * \"{body}\"\n  Assets:Cash -5 CNY\n  Expenses:Food\n");
                let _ = parse(&content, None);
                let _ = parse(&format!("2024-01-01 query \"q\" \"{body}"), None);
            }
        }
    }

    /// The metadata values beancount writes without quotes, which a zhang file reads the same way,
    /// each kept as written (#475).
    mod metadata_values {
        use zhang_ast::{Directive, ZhangString};

        use crate::data_type::text::parser::test::get_txn;
        use crate::data_type::text::parser::{is_valid_bare_meta_value, parse};

        /// Every kind of value, as `(key, text)`, once with a trailing comment.
        const VALUES: &[(&str, &str)] = &[
            ("account", "Assets:Bank:Checking"),
            ("currency", "USD"),
            ("number", "10.50"),
            ("negative", "-3"),
            ("grouped", "1,000.00"),
            ("expression", "(1 + 2) * 3"),
            ("amount", "10 USD"),
            ("amount-expression", "1 + 2 USD"),
            ("date", "2024-01-10"),
            ("tag", "#trip"),
            ("yes", "TRUE"),
            ("no", "FALSE"),
            ("nothing", "NULL"),
        ];

        fn lines(indent: &str) -> String {
            VALUES
                .iter()
                .enumerate()
                .map(|(index, (key, value))| {
                    let comment = if index % 2 == 0 { " ; a comment" } else { "" };
                    format!("{indent}{key}: {value}{comment}\n")
                })
                .collect()
        }

        fn assert_values(meta: &zhang_ast::Meta, context: &str) {
            for (key, value) in VALUES {
                assert_eq!(meta.get_one(*key), Some(&ZhangString::unquote(*value)), "{context}: {key}");
            }
        }

        #[test]
        fn directive_metadata_keeps_every_bare_value_as_written() {
            let content = format!("2024-01-01 open Assets:Bank:Checking USD\n{}", lines("  "));
            let directives = parse(&content, None).unwrap();
            assert_eq!(directives.len(), 1, "{content}");
            let Directive::Open(open) = &directives[0].data else {
                panic!("expected an open directive, got {:?}", directives[0].data);
            };
            assert_values(&open.meta, "open");
        }

        #[test]
        fn transaction_and_posting_metadata_keep_every_bare_value_as_written() {
            let content = format!(
                "2024-01-10 * \"Transfer\"\n{}  Assets:Cash -10 USD\n{}  Assets:Bank\n",
                lines("  "),
                lines("    ")
            );
            let txn = get_txn(&content);
            assert_values(&txn.meta, "transaction");
            assert_eq!(txn.postings.len(), 2);
            assert_values(&txn.postings[0].meta, "posting");
            assert!(txn.postings[1].meta.clone().get_flatten().is_empty());
        }

        #[test]
        fn a_bare_value_is_read_whole_or_as_a_word() {
            // a word that starts like a number or an account stays a word
            for (text, value) in [("1.0.0", "1.0.0"), ("2024-01-10T10", "2024-01-10T10"), ("Assets", "Assets"), ("10USD", "10USD")] {
                let txn = get_txn(&format!("2024-01-10 * \"x\"\n  k: {text}\n  Assets:Cash -10 USD\n  Assets:Bank\n"));
                assert_eq!(txn.meta.get_one("k"), Some(&ZhangString::unquote(value)), "{text}");
            }
            // the comments of a zhang file still follow a bare value
            for comment in ["; c", "# c", "// c"] {
                let txn = get_txn(&format!(
                    "2024-01-10 * \"x\"\n  k: 10 {comment}\n  n: Assets:Bank {comment}\n  Assets:Cash -10 USD\n  Assets:Bank\n"
                ));
                assert_eq!(txn.meta.get_one("k"), Some(&ZhangString::unquote("10")), "{comment}");
                assert_eq!(txn.meta.get_one("n"), Some(&ZhangString::unquote("Assets:Bank")), "{comment}");
            }
            // anything after a value that is not a comment is still an error
            for text in ["Assets:Bank 10 USD", "10 USD USD", "1 + 2 3", "Assets:Bank(x)"] {
                assert!(parse(&format!("2024-01-01 open Assets:Cash\n  k: {text}\n"), None).is_err(), "{text}");
            }
        }

        #[test]
        fn an_edited_bare_value_stays_bare_when_it_reads_back() {
            for valid in ["Assets:Bank", "10 USD", "1,000", "(1 + 2) * 3", "1 + 2 USD", "1.0.0"] {
                assert!(is_valid_bare_meta_value(valid), "{valid}");
            }
            for invalid in ["Assets:Bank 10 USD", "10 USD USD", "Assets:", "10 ", " 10", " 1 + 2", "\t10 USD"] {
                assert!(!is_valid_bare_meta_value(invalid), "{invalid:?}");
            }
        }
    }

    /// The checks for names written unquoted run the grammar on the name.
    mod names {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Date, PostingCost, SingleTotalPrice};

        use crate::data_type::text::parser::{
            is_valid_account_name, is_valid_bare_meta_value, is_valid_commodity_name, is_valid_meta_key, is_valid_tag_or_link, is_valid_transaction_flag,
            read_posting_cost, read_posting_price,
        };

        /// A cost or a price given as text (the transaction edit of the server, #473) is read whole by the
        /// posting grammar, in every form the ledger writes, and anything else is not a cost or a price.
        #[test]
        fn posting_costs_and_prices_as_text() {
            let usd = |number: &str| Amount::new(BigDecimal::from_str(number).unwrap(), "USD");
            let cost = |base: Option<&str>, date: Option<&str>, label: Option<&str>, total: bool| PostingCost {
                base: base.map(usd),
                date: date.map(|it| Date::Date(NaiveDate::from_str(it).unwrap())),
                label: label.map(str::to_owned),
                total,
                ..PostingCost::default()
            };
            assert_eq!(read_posting_cost("{150 USD}"), Some(cost(Some("150"), None, None, false)));
            assert_eq!(read_posting_cost("{ 150 USD }"), Some(cost(Some("150"), None, None, false)));
            assert_eq!(read_posting_cost("{{1500 USD}}"), Some(cost(Some("1500"), None, None, true)));
            assert_eq!(read_posting_cost("{}"), Some(cost(None, None, None, false)));
            assert_eq!(
                read_posting_cost("{150 USD, 2024-01-15}"),
                Some(cost(Some("150"), Some("2024-01-15"), None, false))
            );
            assert_eq!(
                read_posting_cost("{150 USD, 2024-01-15, \"lot\"}"),
                Some(cost(Some("150"), Some("2024-01-15"), Some("lot"), false))
            );
            for invalid in [
                "",
                "150 USD",
                "{150 USD",
                "150 USD}",
                "{150 USD} x",
                " {150 USD}",
                "{150}",
                "{{150 USD}",
                "@ 150 USD",
            ] {
                assert_eq!(read_posting_cost(invalid), None, "{invalid:?}");
            }

            assert_eq!(read_posting_price("@ 6 USD"), Some(SingleTotalPrice::Single(usd("6"))));
            assert_eq!(read_posting_price("@6 USD"), Some(SingleTotalPrice::Single(usd("6"))));
            assert_eq!(read_posting_price("@@ 60 USD"), Some(SingleTotalPrice::Total(usd("60"))));
            for invalid in ["", "6 USD", "@ 6", "@ 6 USD x", " @ 6 USD", "@@", "{6 USD}"] {
                assert_eq!(read_posting_price(invalid), None, "{invalid:?}");
            }
        }

        /// #497: every cost spec form beancount reads is read whole as text too: a date or a label
        /// alone, a compound cost and the merge-cost marker. A compound cost with a missing part or in
        /// total braces, and a second cost number, are not costs
        #[test]
        fn cost_spec_forms_as_text() {
            let usd = |number: &str| Amount::new(BigDecimal::from_str(number).unwrap(), "USD");
            let date = Date::Date(NaiveDate::from_str("2024-01-15").unwrap());
            assert_eq!(
                read_posting_cost("{2024-01-15}"),
                Some(PostingCost {
                    date: Some(date),
                    ..PostingCost::default()
                })
            );
            assert_eq!(
                read_posting_cost("{\"lot\"}"),
                Some(PostingCost {
                    label: Some("lot".to_owned()),
                    ..PostingCost::default()
                })
            );
            assert_eq!(
                read_posting_cost("{100 # 5 USD}"),
                Some(PostingCost {
                    base: Some(usd("100")),
                    compound_total: Some(BigDecimal::from(5)),
                    ..PostingCost::default()
                })
            );
            assert_eq!(
                read_posting_cost("{*}"),
                Some(PostingCost {
                    merge: true,
                    ..PostingCost::default()
                })
            );
            for invalid in [
                "{{100 # 5 USD}}",
                "{100 #}",
                "{# 5 USD}",
                "{100 # USD}",
                "{100 USD, 200 USD}",
                "{100 # 5 USD, 100 USD}",
                "{\"lot\", 100 USD, 2024-01-15, 101 USD}",
            ] {
                assert_eq!(read_posting_cost(invalid), None, "{invalid:?}");
            }
        }

        /// The text of a transaction the exporter would not write back (#473): a header comment, a comment line
        /// between postings and a comment after a metadata line; a posting's own comment, metadata and blank
        /// lines are written back.
        #[test]
        fn transactions_with_text_an_edit_would_drop() {
            use crate::data_type::text::parser::transaction_has_unexported_text;
            for kept in [
                "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY",
                "2024-01-15 * \"Bob\" \"coffee\" #tag ^link\n  Assets:Cash -5 CNY ; paid in cash\n  Expenses:Food\n",
                "2024-01-15 * \"Bob\" \"coffee\"\n  note: \"x; y\"\n  ! Assets:Cash -5 CNY\n    rate: 1.5\n\n  Expenses:Food 5 CNY {5 USD} @ 1 USD ; x\n",
                "2024-01-15 txn \"Bob\" \"coffee\"\r\n  Assets:Cash -5 CNY\r\n  Expenses:Food 5 CNY\r\n",
                "2024-01-15 * \"Bob; not a comment\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n",
            ] {
                assert!(!transaction_has_unexported_text(kept), "{kept:?}");
            }
            for dropped in [
                "2024-01-15 * \"Bob\" \"coffee\" ; a comment\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY",
                "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  ; paid in cash\n  Expenses:Food 5 CNY\n",
                "2024-01-15 * \"Bob\" \"coffee\"\n  // a note\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n",
                "2024-01-15 * \"Bob\" \"coffee\"\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n  # commented out: Expenses:Tips 1 CNY\n",
                "2024-01-15 * \"Bob\" \"coffee\"\n  note: \"x\" ; about the note\n  Assets:Cash -5 CNY\n  Expenses:Food 5 CNY\n",
            ] {
                assert!(transaction_has_unexported_text(dropped), "{dropped:?}");
            }
            // not a transaction at all: nothing to warn about
            assert!(!transaction_has_unexported_text(""));
            assert!(!transaction_has_unexported_text("2024-01-15 open Assets:Cash"));
        }

        #[test]
        fn bare_meta_values() {
            for valid in [
                "1.5",
                "-2",
                "2024-01-15",
                "TRUE",
                "USD",
                "#tag",
                "a;b",
                "中文",
                "Assets:Bank",
                "10 USD",
                "1,000",
                "1 + 2",
            ] {
                assert!(is_valid_bare_meta_value(valid), "{valid}");
            }
            for invalid in [
                "",
                "from plugin",
                "a: b",
                "a:b",
                "\"quoted\"",
                "a\"b",
                "(x)",
                "a,b",
                "tab\t",
                "line\n",
                " x",
                " 10",
            ] {
                assert!(!is_valid_bare_meta_value(invalid), "{invalid:?}");
            }
        }

        #[test]
        fn account_names() {
            for valid in ["Assets:Bank", "Expenses:Food:Lunch", "Income:中文", "Liabilities:Card-1", "Equity:a;b"] {
                assert!(is_valid_account_name(valid), "{valid}");
            }
            for invalid in [
                "",
                "Assets",
                "Assets:",
                "Assets::x",
                "Assets:My Bank",
                "Assets:a\"b",
                "Assets:a,b",
                "Bank:X",
                "assets:x",
            ] {
                assert!(!is_valid_account_name(invalid), "{invalid}");
            }
        }

        #[test]
        fn commodity_names() {
            for valid in ["CNY", "VBMPX", "A.B_C-D'E", "usd"] {
                assert!(is_valid_commodity_name(valid), "{valid}");
            }
            for invalid in ["", "1CNY", "US D", "人民币", "CNY!", "C:Y"] {
                assert!(!is_valid_commodity_name(invalid), "{invalid}");
            }
        }

        #[test]
        fn tags_and_links() {
            for valid in ["trip", "trip-2024", "旅行", "a#b", "^x", "a;b"] {
                assert!(is_valid_tag_or_link(valid), "{valid}");
            }
            for invalid in ["", "two words", "a:b", "a,b", "(x)", "a\"b", "tab\t", "line\n"] {
                assert!(!is_valid_tag_or_link(invalid), "{invalid:?}");
            }
        }

        #[test]
        fn meta_keys() {
            for valid in ["receipt", "receipt-no", "a;b", "a#b", "/x"] {
                assert!(is_valid_meta_key(valid), "{valid}");
            }
            for invalid in ["", "receipt no", "a:b", ";x", "#x", "*x", "//x", "\"x\"", "a,b"] {
                assert!(!is_valid_meta_key(invalid), "{invalid:?}");
            }
        }

        #[test]
        fn transaction_flags() {
            for valid in ["*", "!", "#", "&", "?", "%", "P", "C", "A", "txn"] {
                assert!(is_valid_transaction_flag(valid), "{valid}");
            }
            for invalid in ["", "a", " ", "\"", "**", "$", "^", "*!"] {
                assert!(!is_valid_transaction_flag(invalid), "{invalid:?}");
            }
        }
    }
}

#[cfg(test)]
mod span_positions {
    use super::{line_column, parse};

    /// The ledger of #493: a transaction on lines 5 to 7, at bytes 98 to 157.
    const LEDGER: &str = "option \"operating_currency\" \"CNY\"\n\
                          1970-01-01 open Assets:A CNY\n\
                          1970-01-01 open Expenses:Food CNY\n\
                          \n\
                          2024-01-10 \"Lunch\"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY\n";

    /// A span records the line and column where its directive starts, next to its byte offsets.
    #[test]
    fn spans_record_the_line_and_column_of_their_directive() {
        let spans = parse(LEDGER, None).unwrap().into_iter().map(|it| it.span).collect::<Vec<_>>();
        let positions = spans.iter().map(|span| (span.line, span.column, span.start)).collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![(Some(1), Some(1), 0), (Some(2), Some(1), 34), (Some(3), Some(1), 63), (Some(5), Some(1), 98)]
        );
        assert_eq!(spans[3].end, 157);
        assert_eq!(spans[3].content, "2024-01-10 \"Lunch\"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY");
    }

    /// Columns count characters, and lines are counted on from a position already known.
    #[test]
    fn line_column_counts_characters_and_continues_from_a_known_position() {
        let text = "第一行\n  éb\r\nc";
        let b = text.find('b').unwrap();
        assert_eq!(line_column(text, (1, 0), 0), (1, 1));
        assert_eq!(line_column(text, (1, 0), b), (2, 4));
        assert_eq!(line_column(text, (2, text.find("  é").unwrap()), b), (2, 4));
        assert_eq!(line_column(text, (1, 0), text.find('c').unwrap()), (3, 1));
    }
}
