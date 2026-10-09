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
//!
//! The grammar both formats share is zhang-core's ([`zhang_core::data_type::text::parser`]), so
//! both data types read it the same way: the directives they write alike, and comments, metadata,
//! postings and transactions read with beancount's leaves (`BeancountText`).

use std::path::PathBuf;

use itertools::Either;
use nom::branch::alt;
use nom::bytes::complete::{tag, take_while1};
use nom::character::complete::{char, line_ending, space0, space1};
use nom::combinator::{eof, map, map_res, opt, peek, value};
use nom::sequence::{delimited, preceded, terminated, tuple};
use nom::IResult;
use zhang_ast::*;
use zhang_core::data_type::text::parser as zhang;
// the name tests (`test::names`) read these against zhang-core's validators
#[cfg(test)]
use zhang_core::data_type::text::parser::{account_name, meta_key, spaced_tag_or_link, transaction_flag};
use zhang_core::data_type::text::parser::{
    commodity_name, date_only, directive_metas, flag_char, inline_comment, key_value_line, line_column, line_trailer, offset, parse_items, posting_amount,
    quote_string, string, transaction_head, unquote_string_raw, valuable_comment, Leaves, TransactionLine,
};
use zhang_core::utils::read_time;

use crate::directives::{BalanceDirective, BeancountDirective, BeancountOnlyDirective};

/// Error returned when the input cannot be parsed as beancount text: zhang-core's, with a beancount message.
pub use zhang::ParseError;

// ---------------------------------------------------------------------------
// the leaves of a beancount file; the rest of the grammar is zhang-core's, read with them
// ---------------------------------------------------------------------------

/// The leaves where a beancount file differs from a zhang file ([`Leaves`]).
struct BeancountText;

impl Leaves for BeancountText {
    /// `comment_prefix = ";" | "*" | "#" | "//"`: the start of a comment in a beancount file. `*` is
    /// one here, for the org-mode headings beancount ignores at the start of a line, such as
    /// `* Banking`, and not in a zhang file, where zhang-core's parser reads `*` as a flag only. Inside
    /// a transaction a posting is read before a comment, so `* Assets:Cash -10 USD` is a posting flagged
    /// `*` all the same ([`Leaves::posting_flag`]).
    fn comment_prefix(i: &str) -> IResult<&str, &str> {
        alt((tag("//"), tag(";"), tag("*"), tag("#")))(i)
    }

    /// `posting_flag = flag_char space+`: the flag of a posting, before its account, such as the `!`
    /// of `! Assets:Cash -10 USD`. A beancount file takes every flag beancount 3 reads on a posting, as
    /// beancount does, `#` included, which a zhang file reads as a comment. Beancount takes no `txn`
    /// there. The space is required, so an indented `*` or `#` comment such as `*Assets:Cash -10 USD`
    /// stays a comment.
    fn posting_flag(i: &str) -> IResult<&str, Flag> {
        terminated(flag_char, space1)(i)
    }

    /// beancount dates are date-only, in a directive and in the cost of a posting alike; time (when
    /// present) is carried in metadata and re-attached by the caller.
    fn date(i: &str) -> IResult<&str, Date> {
        date_only(i)
    }
}

// ---------------------------------------------------------------------------
// directive bodies
// ---------------------------------------------------------------------------

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

/// zhang's `open`, then the booking method beancount writes after the commodities
fn open_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, mut open) = zhang::open_body(date, i)?;
    let (i, booking) = opt(preceded(space1, booking_method))(i)?;
    if let (Some(booking), Some(meta)) = (booking, open.meta_mut()) {
        meta.insert("booking_method".to_string(), ZhangString::quote(booking));
    }
    Ok((i, Either::Left(open)))
}

/// zhang's `balance` without its `with pad`, which beancount has not
fn balance_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, (account, amount, tolerance)) = zhang::balance_assertion(i)?;
    Ok((
        i,
        Either::Right(BeancountOnlyDirective::Balance(BalanceDirective {
            date,
            account,
            amount,
            tolerance,
            meta: Meta::default(),
        })),
    ))
}

/// `custom` directives. A `custom` whose type is `budget`, `budget-add`, `budget-transfer` or
/// `budget-close` is one of zhang's budget directives (#500): in the form beancount itself
/// accepts, a quoted type, quoted names and, for `budget`, a quoted commodity, as in
/// `custom "budget" "Food" "CNY"` and `custom "budget-add" "Food" 1000 CNY`; or in the unquoted
/// form earlier versions of zhang wrote, `custom budget Food CNY`, which beancount rejects but
/// existing ledgers still hold. A `custom` of a budget's type whose values are not a budget
/// directive's, such as Fava's `custom "budget" Expenses:Coffee "daily" 4.00 EUR`, is a generic
/// custom directive, as it was, and so is every other type.
fn custom_body(date: Date, original: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(original)?;
    let (rest, custom_type) = string(i)?;
    let budget = match custom_type.as_str() {
        "budget" => budget_body(date.clone(), rest),
        "budget-add" => budget_add_body(date.clone(), rest),
        "budget-transfer" => budget_transfer_body(date.clone(), rest),
        "budget-close" => budget_close_body(date.clone(), rest),
        _ => Err(nom::Err::Error(nom::error::Error::new(rest, nom::error::ErrorKind::Tag))),
    };
    match budget {
        Ok(directive) => Ok(directive),
        // a malformed escape in a quoted value is reported where it is, as in every string
        Err(nom::Err::Failure(error)) => Err(nom::Err::Failure(error)),
        Err(_) => shared(zhang::custom_body(date, original)),
    }
}

/// The name of a budget: quoted, as beancount requires, or bare, as earlier versions of zhang
/// wrote it.
fn budget_name(i: &str) -> IResult<&str, String> {
    map(string, ZhangString::to_plain_string)(i)
}

/// The commodity of a `custom "budget"`: quoted, `"CNY"`, the only form beancount accepts there,
/// or bare, as earlier versions of zhang wrote it.
fn budget_commodity(i: &str) -> IResult<&str, String> {
    alt((
        map_res(quote_string, |quoted| {
            let name = quoted.to_plain_string();
            let is_commodity = matches!(commodity_name(name.as_str()), Ok(("", _)));
            is_commodity.then_some(name).ok_or(())
        }),
        commodity_name,
    ))(i)
}

/// The end of a budget directive's line, left for the caller: blank, or a comment. A `custom` of
/// a budget's type with more values than the budget directive has is a generic custom directive,
/// not a budget directive with the rest of its line left over.
fn budget_line_end(i: &str) -> IResult<&str, ()> {
    peek(value(
        (),
        tuple((space0, opt(inline_comment::<BeancountText>), alt((value((), line_ending), value((), eof))))),
    ))(i)
}

fn budget_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, name) = budget_name(i)?;
    let (i, _) = space1(i)?;
    let (i, commodity) = budget_commodity(i)?;
    let (i, _) = budget_line_end(i)?;
    Ok((
        i,
        Either::Left(Directive::Budget(Budget {
            date,
            name,
            commodity,
            meta: Meta::default(),
        })),
    ))
}

fn budget_add_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, name) = budget_name(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = posting_amount(i)?;
    let (i, _) = budget_line_end(i)?;
    Ok((
        i,
        Either::Left(Directive::BudgetAdd(BudgetAdd {
            date,
            name,
            amount,
            meta: Meta::default(),
        })),
    ))
}

fn budget_transfer_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, from) = budget_name(i)?;
    let (i, _) = space1(i)?;
    let (i, to) = budget_name(i)?;
    let (i, _) = space1(i)?;
    let (i, amount) = posting_amount(i)?;
    let (i, _) = budget_line_end(i)?;
    Ok((
        i,
        Either::Left(Directive::BudgetTransfer(BudgetTransfer {
            date,
            from,
            to,
            amount,
            meta: Meta::default(),
        })),
    ))
}

fn budget_close_body(date: Date, i: &str) -> IResult<&str, BeancountDirective> {
    let (i, _) = space1(i)?;
    let (i, name) = budget_name(i)?;
    let (i, _) = budget_line_end(i)?;
    Ok((
        i,
        Either::Left(Directive::BudgetClose(BudgetClose {
            date,
            name,
            meta: Meta::default(),
        })),
    ))
}

/// A directive both formats write alike, read by zhang-core's grammar.
fn shared(result: IResult<&str, Directive>) -> IResult<&str, BeancountDirective> {
    result.map(|(i, directive)| (i, Either::Left(directive)))
}

/// A dated directive: parse the shared `date keyword` prefix then dispatch on the
/// keyword. Fails (so the caller can try a transaction) on an unknown keyword.
fn dated_directive(original: &str) -> IResult<&str, BeancountDirective> {
    let (i, date) = terminated(BeancountText::date, space1)(original)?;
    let (rest, keyword) = take_while1(|c: char| c.is_ascii_lowercase())(i)?;
    match keyword {
        "open" => open_body(date, rest),
        "close" => shared(zhang::close_body(date, rest)),
        "note" => shared(zhang::note_body(date, rest)),
        "balance" => balance_body(date, rest),
        "pad" => shared(zhang::pad_body(date, rest)),
        "document" => shared(zhang::document_body(date, rest)),
        "price" => shared(zhang::price_body(date, rest)),
        "event" => shared(zhang::event_body(date, rest)),
        "query" => shared(zhang::query_body(date, rest)),
        "commodity" => shared(zhang::commodity_body(date, rest)),
        "custom" => custom_body(date, rest),
        _ => Err(nom::Err::Error(nom::error::Error::new(original, nom::error::ErrorKind::Tag))),
    }
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
    let (i, directive) = alt((map(zhang::plugin_directive, Either::Left), dated_directive))(i)?;
    let (i, metas) = directive_metas::<BeancountText>(i)?;
    let directive = match metas {
        Some(meta) => set_meta(directive, meta),
        None => directive,
    };
    Ok((i, directive))
}

/// A transaction of a beancount file ([`transaction_head`]).
fn transaction(original: &str) -> IResult<&str, BeancountDirective> {
    let (i, (mut transaction, lines)) = transaction_head::<BeancountText>(original)?;
    // as in beancount, a metadata line before the first posting belongs to the
    // transaction and one after a posting to that posting, however it is indented
    let mut posting_indent = 0;
    let mut posting_times = vec![];
    for (indent, line) in lines {
        match line {
            TransactionLine::Posting(posting) => {
                transaction.postings.push(*posting);
                posting_indent = indent;
            }
            TransactionLine::Meta((key, value)) => match transaction.postings.len().checked_sub(1) {
                Some(posting_index) => {
                    if key == "time" {
                        let deeper = indent > posting_indent;
                        posting_times.push(PostingTime { posting_index, deeper });
                    }
                    transaction.postings[posting_index].meta.insert(key, value)
                }
                None => transaction.meta.insert(key, value),
            },
            TransactionLine::Other => {}
        }
    }
    lift_trailing_time(&mut transaction, &posting_times);
    Ok((i, Either::Left(Directive::Transaction(transaction))))
}

/// A `time` metadata line given to a posting.
struct PostingTime {
    posting_index: usize,
    /// whether the line is indented deeper than its posting line
    deeper: bool,
}

/// Older zhang wrote the metadata of a transaction, `time` included, after its postings
/// and at their indentation, where beancount reads it as metadata of the last posting.
/// Such a `time` is the transaction's again when nothing says it is the posting's: the
/// transaction has no `time` of its own, the line is the only `time` of any posting, it
/// belongs to the last posting without being indented deeper than it, and it is a time of
/// day. Any other metadata stays on the posting, as beancount reads it.
fn lift_trailing_time(transaction: &mut Transaction, posting_times: &[PostingTime]) {
    let [PostingTime { posting_index, deeper: false }] = posting_times else {
        return;
    };
    if transaction.meta.get_one("time").is_some() || *posting_index + 1 != transaction.postings.len() {
        return;
    }
    let posting = &mut transaction.postings[*posting_index];
    if posting.meta.get_one("time").is_some_and(|time| read_time(time.as_str()).is_some()) {
        if let Some(time) = posting.meta.pop_one("time") {
            transaction.meta.insert("time".to_owned(), time);
        }
    }
}

fn content_item(i: &str) -> IResult<&str, Option<BeancountDirective>> {
    alt((
        map(terminated(zhang::option_directive, line_trailer::<BeancountText>), |it| Some(Either::Left(it))),
        map(terminated(zhang::include_directive, line_trailer::<BeancountText>), |it| Some(Either::Left(it))),
        map(terminated(push_tag_directive, line_trailer::<BeancountText>), Some),
        map(terminated(pop_tag_directive, line_trailer::<BeancountText>), Some),
        map(terminated(push_meta_directive, line_trailer::<BeancountText>), Some),
        map(terminated(pop_meta_directive, line_trailer::<BeancountText>), Some),
        map(valuable_comment::<BeancountText>, |content| {
            Some(Either::Left(Directive::Comment(Comment { content })))
        }),
        map(metable_item, Some),
        map(transaction, Some),
    ))(i)
}

fn error_at(original: &str, rest: &str, message: &str) -> ParseError {
    let (line, column) = line_column(original, (1, 0), offset(original, rest));
    ParseError {
        message: format!("failed to parse beancount file: {} at line {}, column {}", message, line, column),
    }
}

/// Parse a full beancount text file into a list of spanned directives.
pub fn parse(input_str: &str, file: impl Into<Option<PathBuf>>) -> Result<Vec<Spanned<BeancountDirective>>, ParseError> {
    parse_items(input_str, file.into(), content_item, error_at)
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
    mod note_and_document_tags {
        use std::collections::HashSet;

        use zhang_ast::Directive;

        use crate::parser::test::get_left_directive;

        fn set(items: &[&str]) -> Option<HashSet<String>> {
            Some(items.iter().map(|it| (*it).to_owned()).collect())
        }

        #[test]
        fn notes_and_documents_keep_their_tags_and_links() {
            let Directive::Note(note) = get_left_directive("2020-01-10 note Assets:Bank \"x\" #t1 ^ln\n") else {
                panic!()
            };
            assert_eq!((note.tags, note.links), (set(&["t1"]), set(&["ln"])));
            let Directive::Document(document) = get_left_directive("2020-01-10 document Assets:Bank \"a.pdf\" #t1 #t2\n") else {
                panic!()
            };
            assert_eq!((document.tags, document.links), (set(&["t1", "t2"]), None));
            let Directive::Note(note) = get_left_directive("2020-01-10 note Assets:Bank \"x\"\n") else {
                panic!()
            };
            assert_eq!((note.tags, note.links), (None, None));
        }
    }
    mod tag {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Account, Date, Directive, Pad};

        use crate::directives::{BalanceDirective, BeancountOnlyDirective};
        use crate::parser::test::{get_left_directive, get_right_directive};

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
            let directive = get_left_directive("1970-01-01 pad Assets:BankAccount Assets:BankAccount2");
            assert_eq!(
                Directive::Pad(Pad {
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

        #[test]
        fn should_support_every_beancount_flag_on_the_transaction() {
            // beancount 3.2.3 reads `&`, `?` and `%` as flags too
            for flag in ["&", "?", "%"] {
                let trx = get_txn(&format!("2022-06-02 {flag} \"x\"\n  Assets:Card -1 USD\n  Expenses:Food\n"));
                assert_eq!(trx.flag, Some(Flag::Custom(flag.to_string())));
            }
        }
    }

    /// The cost spec forms of #497, read through zhang-core's grammar with beancount's dates.
    mod cost {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Date, PostingCost};

        use crate::parser::parse;
        use crate::parser::test::get_txn;

        /// #497: every cost spec form bean-check 3.2.3 reads, read through zhang-core's grammar with
        /// beancount's dates: a date or a label alone, the components in any order, the compound
        /// cost `P # T CUR` and the merge-cost marker `*`
        #[test]
        fn every_cost_spec_form_is_read() {
            let usd = |number: &str| Amount::new(BigDecimal::from_str(number).unwrap(), "USD");
            let date = Date::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 10).unwrap());
            let cases = [
                (
                    "{100 USD}",
                    PostingCost {
                        base: Some(usd("100")),
                        ..Default::default()
                    },
                ),
                (
                    "{{1000 USD}}",
                    PostingCost {
                        base: Some(usd("1000")),
                        total: true,
                        ..Default::default()
                    },
                ),
                ("{}", PostingCost::default()),
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
                    "{100 # 5 USD}",
                    PostingCost {
                        base: Some(usd("100")),
                        compound_total: Some(BigDecimal::from(5)),
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
                    "{\"a\", 2024-01-10, 100 USD}",
                    PostingCost {
                        base: Some(usd("100")),
                        date: Some(date.clone()),
                        label: Some("a".to_owned()),
                        ..Default::default()
                    },
                ),
            ];
            for (spec, expected) in cases {
                let txn = get_txn(&format!(
                    "2024-02-01 * \"sell\"\n  Assets:Broker -1 HOOL {spec} @ 120 USD\n  Assets:Cash 120 USD\n"
                ));
                assert_eq!(txn.postings[0].cost, Some(expected), "{spec}");
                assert!(txn.postings[0].price.is_some(), "{spec}");
            }
        }

        /// a compound cost in total braces, which beancount reads with an error (dropping the per-unit
        /// part), does not read, nor does a second cost number, which beancount reports
        #[test]
        fn a_compound_total_cost_and_a_second_cost_number_are_rejected() {
            for cost in ["{{100 # 5 USD}}", "{100 USD, 101 USD}"] {
                let text = format!("2024-02-01 * \"buy\"\n  Assets:Broker 10 HOOL {cost}\n  Assets:Cash -1005 USD\n");
                assert!(parse(&text, None).is_err(), "{cost}");
            }
        }
    }

    /// A posting can carry its own flag before its account (#474). Every case here was checked with
    /// beancount 3.2.3, which reads the same flags.
    mod posting_flags {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Flag, PostingCost, SingleTotalPrice, ZhangString};

        use crate::parser::parse;
        use crate::parser::test::get_txn;

        fn amount(number: &str, commodity: &str) -> Amount {
            Amount::new(BigDecimal::from_str(number).unwrap(), commodity)
        }

        #[test]
        fn a_posting_keeps_its_own_flag() {
            let txn = get_txn(indoc! {r#"
                2024-01-10 * "Lunch"
                  ! Assets:Cash  -10 USD
                  * Expenses:Food 6 USD
                  Expenses:Drinks 4 USD
            "#});
            assert_eq!(txn.flag, Some(Flag::Okay));
            let flags = txn.postings.iter().map(|it| it.flag.clone()).collect::<Vec<_>>();
            assert_eq!(flags, vec![Some(Flag::Warning), Some(Flag::Okay), None]);
            let accounts = txn.postings.iter().map(|it| it.account.name()).collect::<Vec<_>>();
            assert_eq!(accounts, vec!["Assets:Cash", "Expenses:Food", "Expenses:Drinks"]);
            assert_eq!(txn.postings[0].units, Some(amount("-10", "USD")));
        }

        #[test]
        fn every_beancount_posting_flag_is_read() {
            for (written, flag) in [
                ("!", Flag::Warning),
                ("*", Flag::Okay),
                ("#", Flag::Custom("#".to_owned())),
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
                  * Assets:Bank 1000 USD
                  Income:Gains
            "#});
            let sold = &txn.postings[0];
            assert_eq!(sold.flag, Some(Flag::Warning));
            assert_eq!(sold.units, Some(amount("-5", "AAPL")));
            assert_eq!(sold.cost, Some(PostingCost::default()));
            assert_eq!(sold.price, Some(SingleTotalPrice::Single(amount("200", "USD"))));
            assert_eq!(sold.comment.as_deref(), Some("check the lot"));
            // in beancount every metadata line after a posting is the posting's
            assert_eq!(sold.meta.get_one("receipt"), Some(&ZhangString::quote("r-1")));
            assert_eq!(txn.postings[1].flag, Some(Flag::Okay));
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
            // beancount 3.2.3 reads `!Assets:Cash` as a flagged posting; zhang requires the space
            assert!(parse("2024-01-10 * \"Lunch\"\n  !Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
            assert!(parse("2024-01-10 * \"Lunch\"\n  ?Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
            // `txn` is a transaction flag only: beancount 3.2.3 reports a syntax error
            assert!(parse("2024-01-10 * \"Lunch\"\n  txn Assets:Cash -10 USD\n  Expenses:Food\n", None).is_err());
        }
    }
    mod query {
        use chrono::NaiveDate;
        use indoc::indoc;
        use zhang_ast::{Date, Directive, Query, ZhangString};

        use crate::parser::parse;
        use crate::parser::test::get_left_directive;

        fn get_query(content: &str) -> Query {
            match get_left_directive(content) {
                Directive::Query(query) => query,
                other => unreachable!("should get query, but found {:?}", other),
            }
        }

        #[test]
        fn should_parse_query() {
            let query = get_query(indoc! {r#"
                            2014-07-09 query "france-balances" "SELECT account, sum(position) WHERE 'trip-france-2014' in tags"
                        "#});
            assert_eq!(query.date, Date::Date(NaiveDate::from_ymd_opt(2014, 7, 9).unwrap()));
            assert_eq!(query.name, ZhangString::quote("france-balances"));
            assert_eq!(
                query.query_string,
                ZhangString::quote("SELECT account, sum(position) WHERE 'trip-france-2014' in tags")
            );
        }

        #[test]
        fn should_parse_query_with_meta() {
            let query = get_query(indoc! {r#"
                            2014-07-09 query "cash" "SELECT account" ; a comment
                              owner: "alice"
                              rank: 2
                        "#});
            assert_eq!(query.query_string, ZhangString::quote("SELECT account"));
            assert_eq!(query.meta.get_one("owner"), Some(&ZhangString::quote("alice")));
            assert_eq!(query.meta.get_one("rank"), Some(&ZhangString::unquote("2")));
        }

        #[test]
        fn should_parse_multi_line_query() {
            let query = get_query("2014-07-09 query \"multi\" \"\n  SELECT account\n  WHERE account ~ 'Cash'\n\"\n");
            assert_eq!(query.query_string, ZhangString::quote("\n  SELECT account\n  WHERE account ~ 'Cash'\n"));
        }

        #[test]
        fn should_reject_query_without_text() {
            assert!(parse("2014-07-09 query \"name\"\n", None).is_err());
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

        /// The form beancount accepts (#500): a quoted type and quoted names, a quoted commodity for
        /// `budget`, and a plain amount for `budget-add` and `budget-transfer`. Each is the same
        /// directive as zhang's own budget directive, and as the unquoted form earlier versions of
        /// zhang wrote in beancount files, which is still read.
        #[test]
        fn should_read_the_quoted_form_beancount_accepts_as_zhangs_budget_directives() {
            let cases = [
                (
                    "2024-01-01 budget Food CNY\n  alias: \"外食\"\n  category: \"生活开销｜55%\"\n",
                    "2024-01-01 custom \"budget\" \"Food\" \"CNY\"\n  alias: \"外食\"\n  category: \"生活开销｜55%\"\n",
                    "2024-01-01 custom budget Food CNY\n  alias: \"外食\"\n  category: \"生活开销｜55%\"\n",
                ),
                (
                    "2024-01-01 budget-add Food 2000 CNY\n",
                    "2024-01-01 custom \"budget-add\" \"Food\" 2000 CNY\n",
                    "2024-01-01 custom budget-add Food 2000 CNY\n",
                ),
                (
                    "2024-01-20 budget-transfer Fun Food 100.5 CNY\n",
                    "2024-01-20 custom \"budget-transfer\" \"Fun\" \"Food\" 100.5 CNY\n",
                    "2024-01-20 custom budget-transfer Fun Food 100.5 CNY\n",
                ),
                (
                    "2024-12-31 budget-close Food\n",
                    "2024-12-31 custom \"budget-close\" \"Food\"\n",
                    "2024-12-31 custom budget-close Food\n",
                ),
            ];
            for (zhang, quoted, unquoted) in cases {
                let expected = zhang_core::data_type::text::parser::parse(zhang, None::<std::path::PathBuf>)
                    .unwrap()
                    .pop()
                    .unwrap()
                    .data;
                assert!(!matches!(expected, Directive::Custom(..)), "{}", zhang);
                assert_eq!(get_left_directive(quoted), expected, "{}", quoted);
                assert_eq!(get_left_directive(unquoted), expected, "{}", unquoted);
            }
        }

        /// A quoted name holds what a bare word cannot, and a trailing comment is no part of the
        /// directive.
        #[test]
        fn should_read_quoted_names_with_spaces_and_a_trailing_comment() {
            let directive = get_left_directive("2024-01-01 custom \"budget\" \"Eating out: lunch\" \"CNY\" ; weekdays\n");
            let Directive::Budget(budget) = directive else { panic!("{:?}", directive) };
            assert_eq!((budget.name.as_str(), budget.commodity.as_str()), ("Eating out: lunch", "CNY"));
            let directive = get_left_directive("2024-01-20 custom \"budget-transfer\" \"Eating out\" \"Fun money\" 100 CNY\n");
            let Directive::BudgetTransfer(transfer) = directive else {
                panic!("{:?}", directive)
            };
            assert_eq!((transfer.from.as_str(), transfer.to.as_str()), ("Eating out", "Fun money"));
        }

        /// A `custom` of a budget's type whose values are not a budget directive's stays a generic
        /// custom directive, as it was: Fava's budgets, a quoted amount, more or fewer values.
        #[test]
        fn should_keep_a_custom_budget_of_another_shape_a_custom_directive() {
            for line in [
                "2024-01-01 custom \"budget\" Expenses:Coffee \"daily\" 4.00 EUR\n",
                "2024-01-01 custom \"budget\" \"Food\" \"CNY\" \"monthly\"\n",
                "2024-01-01 custom \"budget\" \"Food\"\n",
                "2024-01-01 custom \"budget\" \"Food\" \"not a commodity\"\n",
                "2024-01-01 custom \"budget-add\" \"Food\" \"2000 CNY\"\n",
                "2024-01-01 custom \"budget-close\" \"Food\" \"CNY\"\n",
            ] {
                let directive = get_left_directive(line);
                assert!(matches!(directive, Directive::Custom(..)), "{}: {:?}", line, directive);
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

    /// String escaping (issue #442): the beancount data type reads strings with the
    /// same rules as the zhang one, see `zhang_core::utils::string_`.
    mod escaping {
        use zhang_ast::Directive;

        use crate::parser::parse;
        use crate::parser::test::{get_left_directive, get_txn};

        fn narration(quoted: &str) -> String {
            let content = format!("2024-01-01 * {quoted}\n  Assets:Cash -5 CNY\n  Expenses:Food\n");
            get_txn(&content).narration.unwrap().to_plain_string()
        }

        fn query_text(content: &str) -> String {
            match get_left_directive(content) {
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
            assert_eq!(query_text(r#"2014-01-01 query "x" "SELECT narration WHERE narration ~ '\\d+'""#), expected);
        }

        #[test]
        fn should_read_strings_written_by_the_new_exporter() {
            assert_eq!(narration(r#""coffee $5""#), "coffee $5");
            assert_eq!(narration("\"SELECT\u{a0}account\u{2028}😀 你好\""), "SELECT\u{a0}account\u{2028}😀 你好");
            assert_eq!(narration(r#""a \"quote\" and a \\ backslash""#), r#"a "quote" and a \ backslash"#);
            assert_eq!(narration(r#""two\nlines\u0007""#), "two\nlines\u{07}");
        }

        #[test]
        fn should_read_escapes_written_by_older_versions() {
            assert_eq!(narration(r#""coffee \$5""#), "coffee $5");
            assert_eq!(narration(r#""run \`ls\`""#), "run `ls`");
            assert_eq!(narration(r#""a\u{a0}b""#), "a\u{a0}b");
            assert_eq!(narration(r#""smile \u{1F600}""#), "smile 😀");
            assert_eq!(query_text(r#"2014-01-01 query "q" "SELECT\u{a0}account""#), "SELECT\u{a0}account");
        }

        #[test]
        fn should_report_a_malformed_escape_at_the_escape() {
            for escape in [r"\u{110000}", r"\u{D800}", r"\uZZZZ", r"\u{}", r"\uD800"] {
                let line = format!(r#"2024-01-01 note Assets:Cash "bad {escape} escape""#);
                let column = line.find('\\').unwrap() + 1;
                let error = parse_error(&format!("2024-01-01 open Assets:Cash\n{line}\n"));
                assert_eq!(
                    error,
                    format!("failed to parse beancount file: invalid escape sequence at line 2, column {column}"),
                    "{escape}"
                );
            }
        }

        #[test]
        fn should_keep_indented_comment_like_lines_out_of_metadata() {
            for prefix in [";", "#", "*", "//"] {
                let line = format!("  {prefix}path: \"C:\\Users\\me\"");
                let txn = get_txn(&format!("2024-01-01 * \"x\"\n  Assets:Cash -5 CNY\n{line}\n  Expenses:Food\n"));
                assert!(txn.meta.get_one(&format!("{prefix}path")).is_none(), "{line}");
                assert_eq!(txn.postings.len(), 2, "{line}");

                let directives = parse(&format!("2024-01-01 open Assets:Cash\n{line}\n"), None).unwrap();
                assert_eq!(directives.len(), 2, "{line}");
                let itertools::Either::Left(Directive::Open(open)) = &directives[0].data else {
                    panic!("expected an open directive, got {:?}", directives[0].data);
                };
                assert!(open.meta.get_one(&format!("{prefix}path")).is_none(), "{line}");
                assert!(matches!(directives[1].data, itertools::Either::Left(Directive::Comment(_))), "{line}");
            }
        }

        #[test]
        fn should_still_read_plain_and_quoted_metadata_keys() {
            let directives = parse("2024-01-01 open Assets:Cash\n  path: \"a\"\n  \";path\": \"b\"\n  a;b: \"c\"\n", None).unwrap();
            assert_eq!(directives.len(), 1);
            let itertools::Either::Left(Directive::Open(open)) = &directives[0].data else {
                panic!("expected an open directive, got {:?}", directives[0].data);
            };
            assert_eq!(open.meta.get_one("path").map(|it| it.as_str()), Some("a"));
            assert_eq!(open.meta.get_one(";path").map(|it| it.as_str()), Some("b"));
            assert_eq!(open.meta.get_one("a;b").map(|it| it.as_str()), Some("c"));
        }

        #[test]
        fn should_reject_an_unterminated_string_without_panicking() {
            for content in [
                "2024-01-01 note Assets:Cash \"abc\\",
                "2024-01-01 note Assets:Cash \"abc\\\"\n",
                "2024-01-01 open Assets:Cash\n2024-01-01 note Assets:Cash \"abc\\\"\n2024-01-02 open Assets:Bank\n",
            ] {
                assert!(
                    parse_error(content).starts_with("failed to parse beancount file: unexpected input at line"),
                    "{content:?}"
                );
            }
        }
    }

    /// An arithmetic expression without a value, such as `1/0`, made the parser panic (beancount 3.2.3 crashes on it).
    /// It is an error at the operand that makes it so, wherever a number is written, as in a zhang file.
    mod arithmetic {
        use crate::parser::parse;

        #[test]
        fn a_division_by_zero_is_an_error_at_its_divisor() {
            let txn = |posting: &str| format!("2024-01-02 * \"x\"\n  {posting}\n  Assets:Cash\n");
            let cases = [
                (txn("Expenses:Food 1/0 CNY"), 2, 19),
                (txn("Expenses:Food 10 / (2 - 2) CNY"), 2, 22),
                (txn("Assets:Stock 1 STK {1/0 USD}"), 2, 25),
                (txn("Assets:Stock 1 STK @ 1/0 USD"), 2, 26),
                ("2024-01-02 balance Assets:Cash 1/0 CNY\n".to_owned(), 1, 34),
                ("2024-01-02 price STK 1/0 USD\n".to_owned(), 1, 24),
                ("2024-01-02 custom \"budget-add\" Food 1/0 CNY\n".to_owned(), 1, 39),
                ("2024-01-02 open Assets:Cash\n  ratio: 1/0\n".to_owned(), 2, 12),
            ];
            for (content, line, column) in cases {
                assert_eq!(
                    parse(&content, None).expect_err(&content).to_string(),
                    format!("failed to parse beancount file: division by zero at line {line}, column {column}"),
                    "{content:?}"
                );
            }
        }

        #[test]
        fn an_operand_out_of_range_is_an_error() {
            let content = "2024-01-02 price STK 1e-9223372036854775807 * 1e-9223372036854775807 USD\n";
            assert_eq!(
                parse(content, None).expect_err(content).to_string(),
                "failed to parse beancount file: number out of range at line 1, column 47"
            );
        }
    }

    /// The metadata values beancount reads without quotes (#475): an account, a currency, a
    /// number or an arithmetic expression, an amount, a date, a tag, `TRUE`, `FALSE` and `NULL`,
    /// each kept as it is written, on a transaction, on a posting and on the other directives.
    mod metadata_values {
        use itertools::Either;
        use zhang_ast::{Directive, Meta, ZhangString};

        use crate::directives::BeancountOnlyDirective;
        use crate::parser::parse;
        use crate::parser::test::{get_right_directive, get_txn};

        /// Every kind of value, as `(key, text)`.
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

        /// The metadata lines of every value at `indent`, every other one with a trailing comment.
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

        fn assert_values(meta: &Meta, context: &str) {
            for (key, value) in VALUES {
                assert_eq!(meta.get_one(*key), Some(&ZhangString::unquote(*value)), "{context}: {key}");
            }
        }

        #[test]
        fn transaction_and_posting_metadata_keep_every_bare_value_as_written() {
            // the account of the issue, on the transaction, and every kind on a posting
            let txn = get_txn("2024-01-10 * \"Transfer\"\n  counterpart: Assets:Bank\n  Assets:Cash  -10 USD\n  Assets:Bank\n");
            assert_eq!(txn.meta.get_one("counterpart"), Some(&ZhangString::unquote("Assets:Bank")));
            assert_eq!(txn.postings.len(), 2);

            let content = format!(
                "2024-01-10 * \"Transfer\"\n{}  Assets:Cash  -10 USD\n{}  Assets:Bank\n",
                lines("  "),
                lines("    ")
            );
            let txn = get_txn(&content);
            assert_values(&txn.meta, "transaction");
            assert_eq!(txn.postings.len(), 2, "{content}");
            assert_values(&txn.postings[0].meta, "posting");
            assert!(txn.postings[1].meta.clone().get_flatten().is_empty());
        }

        #[test]
        fn directive_metadata_keeps_every_bare_value_as_written() {
            let headers = [
                "2024-01-01 open Assets:Bank:Checking USD",
                "2024-01-01 close Assets:Bank:Checking",
                "2024-01-01 commodity USD",
                "2024-01-01 note Assets:Bank:Checking \"a note\"",
                "2024-01-01 document Assets:Bank:Checking \"a.pdf\"",
                "2024-01-01 price USD 1 USD",
                "2024-01-01 event \"location\" \"home\"",
                "2024-01-01 custom \"note\" \"x\"",
                "2024-01-01 query \"q\" \"SELECT account\"",
                "2024-01-01 pad Assets:Bank:Checking Assets:Cash",
                "plugin \"beancount.plugins.auto\"",
            ];
            for header in headers {
                let content = format!("{header}\n{}", lines("  "));
                let mut directives = parse(&content, None).unwrap_or_else(|error| panic!("{header}: {error}"));
                assert_eq!(directives.len(), 1, "{header}");
                let Either::Left(directive) = directives.pop().unwrap().data else {
                    panic!("{header}: expected a zhang directive");
                };
                let meta = match &directive {
                    Directive::Open(it) => &it.meta,
                    Directive::Close(it) => &it.meta,
                    Directive::Commodity(it) => &it.meta,
                    Directive::Note(it) => &it.meta,
                    Directive::Document(it) => &it.meta,
                    Directive::Price(it) => &it.meta,
                    Directive::Event(it) => &it.meta,
                    Directive::Custom(it) => &it.meta,
                    Directive::Query(it) => &it.meta,
                    Directive::Pad(it) => &it.meta,
                    Directive::Plugin(it) => &it.meta,
                    other => panic!("{header}: unexpected {other:?}"),
                };
                assert_values(meta, header);
            }
            // the beancount-only balance
            let header = "2024-01-01 balance Assets:Bank:Checking 10 USD";
            let BeancountOnlyDirective::Balance(balance) = get_right_directive(&format!("{header}\n{}", lines("  "))) else {
                panic!("{header}: expected a balance");
            };
            assert_values(&balance.meta, header);
        }

        #[test]
        fn pushmeta_takes_a_bare_value() {
            let BeancountOnlyDirective::PushMeta(key, value) = get_right_directive("pushmeta counterpart: Assets:Bank\n") else {
                panic!("expected pushmeta");
            };
            assert_eq!((key.as_str(), value), ("counterpart", ZhangString::unquote("Assets:Bank")));
            let BeancountOnlyDirective::PushMeta(key, value) = get_right_directive("pushmeta limit: 10 USD ; a comment\n") else {
                panic!("expected pushmeta");
            };
            assert_eq!((key.as_str(), value), ("limit", ZhangString::unquote("10 USD")));
        }

        #[test]
        fn a_bare_value_ends_at_the_end_of_the_line_or_a_comment() {
            // more than one value is still an error, as in beancount
            for text in ["Assets:Bank 10 USD", "10 USD USD", "Assets:Bank Assets:Cash", "1 + 2 3"] {
                assert!(parse(&format!("2024-01-01 open Assets:Cash\n  k: {text}\n"), None).is_err(), "{text}");
            }
            // a quoted value is unchanged
            let txn = get_txn("2024-01-10 * \"Transfer\"\n  counterpart: \"Assets:Bank\"\n  Assets:Cash  -10 USD\n  Assets:Bank\n");
            assert_eq!(txn.meta.get_one("counterpart"), Some(&ZhangString::quote("Assets:Bank")));
        }
    }

    /// The server rejects names that would not read back using zhang-core's checks
    /// (`zhang_core::data_type::text::parser::is_valid_*`); beancount must accept
    /// exactly the same names, so those checks hold for beancount ledgers too.
    mod names {
        use zhang_core::data_type::text::parser::{
            is_valid_account_name, is_valid_commodity_name, is_valid_meta_key, is_valid_tag_or_link, is_valid_transaction_flag,
        };

        use crate::parser::{account_name, commodity_name, meta_key, spaced_tag_or_link, transaction_flag};

        fn reads_all<'a, O>(mut parser: impl FnMut(&'a str) -> nom::IResult<&'a str, O>, text: &'a str) -> bool {
            matches!(parser(text), Ok(("", _)))
        }

        #[test]
        fn beancount_accepts_the_same_unquoted_names_as_zhang() {
            const PIECES: &[&str] = &[
                "Assets", "Expenses", "Equity", "Bank", ":", ":", "a", "Z", "0", "9", ".", "_", "-", "'", " ", "\t", "\n", "\"", "(", ")", ",", ";", "#", "*",
                "/", "//", "^", "!", "txn", "{", "\u{a0}", "中", "😀",
            ];
            let mut state = 0x5eed_0442_u64;
            let mut next = move || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as usize
            };
            for _ in 0..20_000 {
                let len = next() % 6;
                let name: String = (0..len).map(|_| PIECES[next() % PIECES.len()]).collect();
                let name = name.as_str();
                assert_eq!(reads_all(account_name, name), is_valid_account_name(name), "account {name:?}");
                assert_eq!(reads_all(commodity_name, name), is_valid_commodity_name(name), "commodity {name:?}");
                assert_eq!(reads_all(meta_key, name), is_valid_meta_key(name), "metadata key {name:?}");
                let tag = format!("#{name}");
                let reads_tag = matches!(spaced_tag_or_link(&tag), Ok(("", (true, ref read))) if read == name);
                assert_eq!(reads_tag, is_valid_tag_or_link(name), "tag {name:?}");
                // `transaction_flag` reads the spaces before the flag too
                let reads_flag = !name.starts_with([' ', '\t']) && reads_all(transaction_flag, &format!(" {name}"));
                assert_eq!(reads_flag, is_valid_transaction_flag(name), "flag {name:?}");
            }
        }
    }
}
