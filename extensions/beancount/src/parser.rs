use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveTime};
use itertools::{Either, Itertools};
use once_cell::sync::OnceCell;
use pest::{iterators::Pairs, pratt_parser::PrattParser};
use pest_consume::{match_nodes, Error, Parser};
use snailquote::unescape;
use zhang_ast::amount::Amount;
use zhang_ast::utils::multi_value_map::MultiValueMap;
use zhang_ast::*;

use crate::directives::{BalanceDirective, BeancountDirective, BeancountOnlyDirective, PadDirective};

type Result<T> = std::result::Result<T, Error<Rule>>;
type Node<'i> = pest_consume::Node<'i, Rule, ()>;

#[derive(Parser)]
#[grammar = "beancount.pest"]
pub struct BeancountParser;

/// Construct a global [PrattParser] to handle number expressions.
fn pratt_number_parser() -> &'static PrattParser<Rule> {
    static PARSER: OnceCell<PrattParser<Rule>> = OnceCell::new();
    PARSER.get_or_init(|| {
        use pest::pratt_parser::{Assoc::*, Op};
        use Rule::*;
        PrattParser::new()
            .op(Op::infix(add, Left) | Op::infix(subtract, Left))
            .op(Op::infix(multiply, Left) | Op::infix(divide, Left))
            .op(Op::prefix(unary_minus))
    })
}

/// Define parsing rules for [number_expr] nodes.
/// Each expression is calculated in-place and reduced to one [BigDecimal].
fn parse_number_expr(pairs: Pairs<Rule>) -> Result<BigDecimal> {
    pratt_number_parser()
        .map_primary(|primary| match primary.as_rule() {
            Rule::number => Ok(BigDecimal::from_str(primary.as_str()).unwrap()),
            Rule::number_expr => parse_number_expr(primary.into_inner()),
            rule => unreachable!("Unexpected number expr {:?}", rule),
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::add => Ok(lhs? + rhs?),
            Rule::subtract => Ok(lhs? - rhs?),
            Rule::multiply => Ok(lhs? * rhs?),
            Rule::divide => Ok(lhs? / rhs?),
            rule => unreachable!("Unexpected infix operation {:?}", rule),
        })
        .map_prefix(|op, rhs| match op.as_rule() {
            Rule::unary_minus => Ok(-rhs?),
            rule => unreachable!("Unexpected prefix operation {:?}", rule),
        })
        .parse(pairs)
}

#[pest_consume::parser]
impl BeancountParser {
    #[allow(dead_code)]
    fn EOI(_input: Node) -> Result<()> {
        Ok(())
    }
    fn number_expr(input: Node) -> Result<BigDecimal> {
        parse_number_expr(input.into_pair().into_inner())
    }
    fn quote_string(input: Node) -> Result<ZhangString> {
        let string = input.as_str();
        Ok(ZhangString::QuoteString(unescape(string).unwrap()))
    }

    fn unquote_string(input: Node) -> Result<ZhangString> {
        Ok(ZhangString::UnquoteString(input.as_str().to_owned()))
    }

    fn string(input: Node) -> Result<ZhangString> {
        let ret = match_nodes!(
            input.into_children();
            [quote_string(i)] => i,
            [unquote_string(i)] => i
        );
        Ok(ret)
    }
    fn commodity_name(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn account_type(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn account_name(input: Node) -> Result<Account> {
        let r: (String, Vec<String>) = match_nodes!(input.into_children();
            [account_type(a), unquote_string(i)..] => {
                (a, i.map(|it|it.to_plain_string()).collect())
            },

        );
        Ok(Account {
            account_type: AccountType::from_str(&r.0).unwrap(),
            content: format!("{}:{}", &r.0, r.1.join(":")),
            components: r.1,
        })
    }
    fn date(input: Node) -> Result<Date> {
        let datetime: Date = match_nodes!(input.into_children();
            [date_only(d)] => d,
        );
        Ok(datetime)
    }

    fn date_only(input: Node) -> Result<Date> {
        let date = NaiveDate::parse_from_str(input.as_str(), "%Y-%m-%d").unwrap();
        Ok(Date::Date(date))
    }

    fn booking_method(input: Node) -> Result<String> {
        let content = input.as_str();
        Ok(content[1..content.len() - 1].to_string())
    }

    fn plugin(input: Node) -> Result<Directive> {
        let ret: (ZhangString, Vec<ZhangString>) = match_nodes!(input.into_children();
            [string(module), string(values)..] => (module, values.collect()),
        );
        Ok(Directive::Plugin(Plugin { module: ret.0, value: ret.1 }))
    }

    fn option(input: Node) -> Result<Directive> {
        let (key, value) = match_nodes!(input.into_children();
            [string(key), string(value)] => (key, value),
        );
        Ok(Directive::Option(Options { key, value }))
    }
    fn comment(input: Node) -> Result<Directive> {
        Ok(Directive::Comment(Comment {
            content: input.as_str().to_owned(),
        }))
    }

    fn open(input: Node) -> Result<Directive> {
        let ret: (Date, Account, Vec<String>, Option<String>, Meta) = match_nodes!(input.into_children();
            [date(date), account_name(a), commodity_name(commodities).., metas(metas)] => (date, a, commodities.collect(), None, metas),
            [date(date), account_name(a), commodity_name(commodities)..] => (date, a, commodities.collect(), None,Meta::default()),
            [date(date), account_name(a), metas(metas)] => (date, a, vec![], None, metas),

            [date(date), account_name(a), commodity_name(commodities).., booking_method(booking_method), metas(metas)] => (date, a, commodities.collect(), Some(booking_method), metas),
            [date(date), account_name(a), commodity_name(commodities).., booking_method(booking_method)] => (date, a, commodities.collect(), Some(booking_method), Meta::default()),
            [date(date), account_name(a), booking_method(booking_method), metas(metas)] => (date, a, vec![], Some(booking_method), metas),
        );

        let (date, account, commodities, booking_method, mut meta) = ret;
        if let Some(booking_method) = booking_method {
            meta.insert("booking_method".to_string(), ZhangString::quote(booking_method));
        }
        let open = Open {
            date,
            account,
            commodities,
            meta,
        };
        Ok(Directive::Open(open))
    }
    fn close(input: Node) -> Result<Directive> {
        let ret: (Date, Account) = match_nodes!(input.into_children();
            [date(date), account_name(a)] => (date, a)
        );
        Ok(Directive::Close(Close {
            date: ret.0,
            account: ret.1,
            meta: Default::default(),
        }))
    }

    #[allow(dead_code)]
    fn identation(input: Node) -> Result<()> {
        Ok(())
    }

    fn key_value_line(input: Node) -> Result<(String, ZhangString)> {
        let ret: (String, ZhangString) = match_nodes!(input.into_children();
            [string(key), string(value)] => (key.to_plain_string(), value),
        );
        Ok(ret)
    }

    fn metas(input: Node) -> Result<Meta> {
        let ret: Vec<(String, ZhangString)> = match_nodes!(input.into_children();
            [key_value_line(lines)..] => lines.collect(),
        );

        Ok(ret.into_iter().collect())
    }

    fn posting_unit(input: Node) -> Result<(Option<Amount>, Option<(Option<Amount>, Option<Date>, Option<SingleTotalPrice>)>)> {
        let ret: (Option<Amount>, Option<(Option<Amount>, Option<Date>, Option<SingleTotalPrice>)>) = match_nodes!(input.into_children();
            [posting_amount(amount)] => (Some(amount), None),
            [posting_meta(meta)] => (None, Some(meta)),
            [posting_amount(amount), posting_meta(meta)] => (Some(amount), Some(meta)),
        );
        Ok(ret)
    }

    fn posting_cost(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number_expr(amount), commodity_name(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }
    fn posting_total_price(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number_expr(amount), commodity_name(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }
    fn posting_single_price(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number_expr(amount), commodity_name(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }

    fn posting_amount(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number_expr(amount), commodity_name(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }

    fn transaction_flag(input: Node) -> Result<Option<Flag>> {
        Ok(Some(Flag::from_str(input.as_str().trim()).unwrap()))
    }

    fn posting_price(input: Node) -> Result<SingleTotalPrice> {
        let ret: SingleTotalPrice = match_nodes!(input.into_children();
            [posting_total_price(p)] => SingleTotalPrice::Total(p),
            [posting_single_price(p)] => SingleTotalPrice::Single(p),
        );
        Ok(ret)
    }
    fn posting_meta(input: Node) -> Result<(Option<Amount>, Option<Date>, Option<SingleTotalPrice>)> {
        let ret: (Option<Amount>, Option<Date>, Option<SingleTotalPrice>) = match_nodes!(input.into_children();
            [] => (None, None, None),
            [posting_cost(cost)] => (Some(cost), None, None),
            [posting_price(p)] => (None, None, Some(p)),
            [posting_cost(cost), date(d)] => (Some(cost), Some(d), None),
            [posting_cost(cost), posting_price(p)] => (Some(cost), None, Some(p)),
            [posting_cost(cost), date(d), posting_price(p)] => (Some(cost), Some(d), Some(p)),
        );
        Ok(ret)
    }
    fn transaction_posting(input: Node) -> Result<Posting> {
        let ret: (
            Option<Flag>,
            Account,
            Option<(Option<Amount>, Option<(Option<Amount>, Option<Date>, Option<SingleTotalPrice>)>)>,
            Meta,
        ) = match_nodes!(input.into_children();
            [account_name(account_name)] => (None, account_name, None, Meta::default()),
            [account_name(account_name), posting_unit(unit)] => (None, account_name, Some(unit), Meta::default()),
            [transaction_flag(flag), account_name(account_name)] => (flag, account_name, None, Meta::default()),
            [transaction_flag(flag), account_name(account_name), posting_unit(unit)] => (flag, account_name, Some(unit), Meta::default()),

            [account_name(account_name), metas(meta)] => (None, account_name, None, meta),
            [account_name(account_name), posting_unit(unit), metas(meta)] => (None, account_name, Some(unit), meta),
            [transaction_flag(flag), account_name(account_name), metas(meta)] => (flag, account_name, None, meta),
            [transaction_flag(flag), account_name(account_name), posting_unit(unit), metas(meta)] => (flag, account_name, Some(unit), meta),
        );

        let (flag, account, unit, meta) = ret;

        let mut line = Posting {
            flag,
            account,
            units: None,
            cost: None,
            cost_date: None,
            price: None,
            comment: None,
            meta,
        };

        if let Some((amount, meta)) = unit {
            line.units = amount;

            if let Some(meta) = meta {
                line.cost = meta.0;
                line.cost_date = meta.1;
                line.price = meta.2;
            }
        }
        Ok(line)
    }

    fn transaction_line(input: Node) -> Result<(Option<Posting>, Option<(String, ZhangString)>)> {
        let ret: (Option<Posting>, Option<(String, ZhangString)>) = match_nodes!(input.into_children();
            [transaction_posting(posting)] => (Some(posting), None),
            [transaction_posting(posting), valuable_comment(c)] => (Some(posting.set_comment(c)), None),
            [key_value_line(meta)] => (None, Some(meta)),
            [key_value_line(meta),  valuable_comment(_)] => (None, Some(meta)),

        );
        Ok(ret)
    }
    fn transaction_lines(input: Node) -> Result<Vec<(Option<Posting>, Option<(String, ZhangString)>)>> {
        let ret = match_nodes!(input.into_children();
            [transaction_line(lines)..] => lines.collect(),
        );
        Ok(ret)
    }

    fn tag(input: Node) -> Result<String> {
        let ret = match_nodes!(input.into_children();
            [unquote_string(tag)] => tag.to_plain_string(),
        );
        Ok(ret)
    }
    fn link(input: Node) -> Result<String> {
        let ret = match_nodes!(input.into_children();
            [unquote_string(tag)] => tag.to_plain_string(),
        );
        Ok(ret)
    }
    fn tags(input: Node) -> Result<Vec<String>> {
        let ret = match_nodes!(input.into_children();
            [tag(tags)..] => tags.collect(),
        );
        Ok(ret)
    }
    fn links(input: Node) -> Result<Vec<String>> {
        let ret = match_nodes!(input.into_children();
            [link(links)..] => links.collect(),
        );
        Ok(ret)
    }

    fn transaction(input: Node) -> Result<Directive> {
        let ret: (
            Date,
            Option<Flag>,
            Option<ZhangString>,
            Option<ZhangString>,
            Vec<String>,
            Vec<String>,
            Vec<(Option<Posting>, Option<(String, ZhangString)>)>,
        ) = match_nodes!(input.into_children();
            [date(date), quote_string(payee), tags(tags), links(links), transaction_lines(lines)] => (date, None, Some(payee), None, tags, links,lines),
            [date(date), quote_string(payee), quote_string(narration), tags(tags), links(links), transaction_lines(lines)] => (date, None, Some(payee), Some(narration), tags, links,lines),
            [date(date), transaction_flag(flag), tags(tags), links(links), transaction_lines(lines)] => (date, flag, None, None, tags, links, lines),
            [date(date), transaction_flag(flag), quote_string(narration), tags(tags), links(links), transaction_lines(lines)] => (date, flag, None, Some(narration), tags, links, lines),
            [date(date), transaction_flag(flag), quote_string(payee), quote_string(narration), tags(tags), links(links), transaction_lines(lines)] => (date, flag, Some(payee), Some(narration), tags, links,lines),
        );
        let mut transaction = Transaction {
            date: ret.0,
            flag: ret.1,
            payee: ret.2,
            narration: ret.3,
            tags: ret.4.into_iter().collect(),
            links: ret.5.into_iter().collect(),
            postings: vec![],
            meta: MultiValueMap::default(),
        };

        for line in ret.6 {
            match line {
                (Some(trx), None) => {
                    transaction.postings.push(trx);
                }
                (None, Some(meta)) => {
                    transaction.meta.insert(meta.0, meta.1);
                }
                _ => {}
            }
        }

        Ok(Directive::Transaction(transaction))
    }

    fn commodity(input: Node) -> Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [date(date), commodity_name(name)] => (date, name, Meta::default()),
            [date(date), commodity_name(name), metas(meta)] => (date, name, meta),
        );
        Ok(Directive::Commodity(Commodity {
            date: ret.0,
            currency: ret.1,
            meta: ret.2,
        }))
    }

    fn string_or_account(input: Node) -> Result<StringOrAccount> {
        let ret: StringOrAccount = match_nodes!(input.into_children();
            [string(value)] => StringOrAccount::String(value),
            [account_name(value)] => StringOrAccount::Account(value),
        );
        Ok(ret)
    }

    fn custom(input: Node) -> Result<Directive> {
        let ret: (Date, ZhangString, Vec<StringOrAccount>) = match_nodes!(input.into_children();
            [date(date), string(module), string_or_account(options)..] => (date, module, options.collect()),
        );
        Ok(Directive::Custom(Custom {
            date: ret.0,
            custom_type: ret.1,
            values: ret.2,
            meta: Default::default(),
        }))
    }

    fn include(input: Node) -> Result<Directive> {
        let ret: ZhangString = match_nodes!(input.into_children();
            [quote_string(path)] => path,
        );
        let include = Include { file: ret };
        Ok(Directive::Include(include))
    }

    fn note(input: Node) -> Result<Directive> {
        let ret: (Date, Account, ZhangString) = match_nodes!(input.into_children();
            [date(date), account_name(a), string(path)] => (date, a, path),
        );
        Ok(Directive::Note(Note {
            date: ret.0,
            account: ret.1,
            comment: ret.2,
            tags: None,
            links: None,
            meta: Default::default(),
        }))
    }

    fn event(input: Node) -> Result<Directive> {
        let ret: (Date, ZhangString, ZhangString) = match_nodes!(input.into_children();
            [date(date), string(name), string(value)] => (date, name, value),
        );
        Ok(Directive::Event(Event {
            date: ret.0,
            event_type: ret.1,
            description: ret.2,
            meta: Default::default(),
        }))
    }

    fn balance(input: Node) -> Result<BeancountOnlyDirective> {
        let (date, account, amount, commodity): (Date, Account, BigDecimal, String) = match_nodes!(input.into_children();
            [date(date), account_name(name), number_expr(amount), commodity_name(commodity)] => (date, name, amount, commodity),
        );
        Ok(BeancountOnlyDirective::Balance(BalanceDirective {
            date,
            account,
            amount: Amount::new(amount, commodity),
            meta: Default::default(),
        }))
    }
    fn pad(input: Node) -> Result<BeancountOnlyDirective> {
        let (date, name, pad): (Date, Account, Account) = match_nodes!(input.into_children();
            [date(date), account_name(name), account_name(pad)] => (date, name, pad),
        );
        Ok(BeancountOnlyDirective::Pad(PadDirective {
            date,
            account: name,
            pad,
            meta: Default::default(),
        }))
    }

    fn document(input: Node) -> Result<Directive> {
        let ret: (Date, Account, ZhangString) = match_nodes!(input.into_children();
            [date(date), account_name(name), string(path)] => (date, name, path),
        );
        Ok(Directive::Document(Document {
            date: ret.0,
            account: ret.1,
            filename: ret.2,
            tags: None,
            links: None,
            meta: Default::default(),
        }))
    }

    fn price(input: Node) -> Result<Directive> {
        let ret: (Date, String, BigDecimal, String) = match_nodes!(input.into_children();
            [date(date), commodity_name(source), number_expr(price), commodity_name(target)] => (date, source, price, target)
        );
        Ok(Directive::Price(Price {
            date: ret.0,
            currency: ret.1,
            amount: Amount::new(ret.2, ret.3),
            meta: Default::default(),
        }))
    }
    fn push_tag(input: Node) -> Result<BeancountOnlyDirective> {
        let ret: String = match_nodes!(input.into_children();
            [tag(tag)] => tag
        );
        Ok(BeancountOnlyDirective::PushTag(ret))
    }
    fn pop_tag(input: Node) -> Result<BeancountOnlyDirective> {
        let ret: String = match_nodes!(input.into_children();
            [tag(tag)] => tag
        );
        Ok(BeancountOnlyDirective::PopTag(ret))
    }
    fn comment_prefix(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn comment_value(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }

    fn valuable_comment(input: Node) -> Result<String> {
        let content: String = match_nodes!(input.into_children();
            [comment_prefix(_), comment_value(v)] => v,
        );
        Ok(content)
    }
    fn budget(input: Node) -> Result<Directive> {
        let ret: (Date, ZhangString, String, Meta) = match_nodes!(input.into_children();
            [date(date), unquote_string(name), commodity_name(commodity)] => (date, name, commodity, Meta::default()),
            [date(date), unquote_string(name), commodity_name(commodity), metas(metas)] => (date, name, commodity, metas)
        );
        Ok(Directive::Budget(Budget {
            date: ret.0,
            name: ret.1.to_plain_string(),
            commodity: ret.2,
            meta: ret.3,
        }))
    }

    fn budget_close(input: Node) -> Result<Directive> {
        let ret: (Date, ZhangString, Meta) = match_nodes!(input.into_children();
            [date(date), unquote_string(name)] => (date, name, Meta::default()),
            [date(date), unquote_string(name), metas(metas)] => (date, name, metas)
        );
        Ok(Directive::BudgetClose(BudgetClose {
            date: ret.0,
            name: ret.1.to_plain_string(),
            meta: ret.2,
        }))
    }

    fn budget_add(input: Node) -> Result<Directive> {
        let ret: (Date, ZhangString, Amount, Meta) = match_nodes!(input.into_children();
            [date(date), unquote_string(name), posting_amount(amount)] => (date, name, amount, Meta::default()),
            [date(date), unquote_string(name), posting_amount(amount), metas(metas)] => (date, name, amount, metas)
        );
        Ok(Directive::BudgetAdd(BudgetAdd {
            date: ret.0,
            name: ret.1.to_plain_string(),
            amount: ret.2,
            meta: ret.3,
        }))
    }
    fn budget_transfer(input: Node) -> Result<Directive> {
        let ret: (Date, ZhangString, ZhangString, Amount, Meta) = match_nodes!(input.into_children();
            [date(date), unquote_string(from), unquote_string(to), posting_amount(amount)] => (date, from, to, amount, Meta::default()),
            [date(date), unquote_string(from), unquote_string(to), posting_amount(amount), metas(metas)] => (date, from, to, amount, metas)
        );
        Ok(Directive::BudgetTransfer(BudgetTransfer {
            date: ret.0,
            from: ret.1.to_plain_string(),
            to: ret.2.to_plain_string(),
            amount: ret.3,
            meta: ret.4,
        }))
    }
    fn metable_head(input: Node) -> Result<BeancountDirective> {
        let ret: BeancountDirective = match_nodes!(input.into_children();

            [open(item)]            => Either::Left(item),
            [close(item)]           => Either::Left(item),
            [note(item)]            => Either::Left(item),
            [event(item)]           => Either::Left(item),
            [document(item)]        => Either::Left(item),
            [balance(item)]         => Either::Right(item), // balance
            [pad(item)]             => Either::Right(item), // pad
            [price(item)]           => Either::Left(item),
            [commodity(item)]       => Either::Left(item),
            [custom(item)]          => Either::Left(item),
            [budget(item)]          => Either::Left(item),
            [budget_close(item)]    => Either::Left(item),
            [budget_add(item)]      => Either::Left(item),
            [budget_transfer(item)] => Either::Left(item)
        );
        Ok(ret)
    }
    fn empty_space_line(input: Node) -> Result<()> {
        Ok(())
    }

    fn item(input: Node) -> Result<Option<(BeancountDirective, SpanInfo)>> {
        let span = input.as_span();
        let span_info = SpanInfo {
            start: span.start_pos().pos(),
            end: span.end_pos().pos(),
            content: span.as_str().to_string(),
            filename: None,
        };
        let ret: Option<BeancountDirective> = match_nodes!(input.into_children();
            [option(item)]          => Some(Either::Left(item)),
            [plugin(item)]          => Some(Either::Left(item)),
            [include(item)]         => Some(Either::Left(item)),
            [push_tag(item)]        => Some(Either::Right(item)),
            [pop_tag(item)]         => Some(Either::Right(item)),
            [comment(item)]         => Some(Either::Left(item)),
            [valuable_comment(item)] => Some(Either::Left(Directive::Comment(Comment { content:item }))),

            [empty_space_line(_)] => None,

            [metable_head(head)]            => Some(head),
            [metable_head(head), metas(meta)]  => {Some(match head {
                Either::Left(directive) => Either::Left(directive.set_meta(meta)),
                Either::Right(directive) => Either::Right(directive.set_meta(meta)),
            })},

            [transaction(item)]     => Some(Either::Left(item)),

        );
        Ok(ret.map(|it| (it, span_info)))
    }

    fn time_part(input: Node) -> Result<u32> {
        Ok(u32::from_str(input.as_str()).unwrap())
    }

    fn time(input: Node) -> Result<NaiveTime> {
        let (hour, min, sec): (u32, u32, u32) = match_nodes!(input.into_children();
            [time_part(hour), time_part(min), time_part(sec)] => (hour, min, sec),
        );
        Ok(NaiveTime::from_hms_opt(hour, min, sec).expect("not a valid time"))
    }

    fn entry(input: Node) -> Result<Vec<Spanned<BeancountDirective>>> {
        let ret: Vec<(BeancountDirective, SpanInfo)> = match_nodes!(input.into_children();
            [item(items).., _] => items.flatten().collect(),
        );
        Ok(ret
            .into_iter()
            .map(|(directive, span_info)| Spanned {
                data: directive,
                span: span_info,
            })
            .collect_vec())
    }
}

pub fn parse(input_str: &str, file: impl Into<Option<PathBuf>>) -> Result<Vec<Spanned<BeancountDirective>>> {
    let file = file.into();
    let inputs = BeancountParser::parse(Rule::entry, input_str)?;
    let input = inputs.single()?;
    BeancountParser::entry(input).map(|mut directives| {
        directives.iter_mut().for_each(|directive| directive.span.filename = file.clone());
        directives
    })
}
pub fn parse_time(input_str: &str) -> Result<NaiveTime> {
    let inputs = BeancountParser::parse(Rule::time, input_str)?;
    let input = inputs.single()?;
    BeancountParser::time(input)
}

#[cfg(test)]
mod test {

    mod tag {
        use std::str::FromStr;

        use bigdecimal::BigDecimal;
        use chrono::NaiveDate;
        use zhang_ast::amount::Amount;
        use zhang_ast::{Account, Date};

        use crate::directives::{BalanceDirective, BeancountOnlyDirective, PadDirective};
        use crate::parser::parse;

        #[test]
        fn should_support_push_tag() {
            let directive = parse("pushtag #mytag", None).unwrap().pop().unwrap().data.right().unwrap();
            assert_eq!(BeancountOnlyDirective::PushTag("mytag".to_string()), directive);
        }
        #[test]
        fn should_support_pop_tag() {
            let directive = parse("poptag #mytag", None).unwrap().pop().unwrap().data.right().unwrap();
            assert_eq!(BeancountOnlyDirective::PopTag("mytag".to_string()), directive);
        }

        #[test]
        fn should_parse_balance() {
            let directive = parse("1970-01-01 balance Assets:BankAccount 2 CNY", None)
                .unwrap()
                .pop()
                .unwrap()
                .data
                .right()
                .unwrap();
            assert_eq!(
                BeancountOnlyDirective::Balance(BalanceDirective {
                    date: Date::Date(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
                    account: Account::from_str("Assets:BankAccount").unwrap(),
                    amount: Amount::new(BigDecimal::from(2i32), "CNY"),
                    meta: Default::default(),
                }),
                directive
            );
        }
        #[test]
        fn should_parse_pad() {
            let directive = parse("1970-01-01 pad Assets:BankAccount Assets:BankAccount2", None)
                .unwrap()
                .pop()
                .unwrap()
                .data
                .right()
                .unwrap();
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
        use crate::parser::parse;
        use bigdecimal::BigDecimal;
        use indoc::indoc;
        use zhang_ast::Directive;

        use crate::parser::parse;

        #[test]
        fn should_parse_posting_meta() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 "Payee" "Narration"
                              Assets:Bank
                                a: b
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::Transaction(..)));
            if let Directive::Transaction(inner) = directive {
                assert_eq!(inner.postings.first().unwrap().meta.get_one("a").cloned().unwrap().to_plain_string(), "b");
            }
        }

        #[test]
        fn should_parse_with_comment() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 "Payee" "Narration" ; 123123
                              Assets:Bank
                                a: b
                              Assets:Bank ;123213
                              a: b
                              b: c ;123123
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::Transaction(..)));
            if let Directive::Transaction(inner) = directive {
                assert_eq!(inner.postings.first().unwrap().meta.get_one("a").cloned().unwrap().to_plain_string(), "b");
                assert_eq!(inner.postings.get(1).unwrap().comment.as_ref().unwrap(), "123213");
            }
        }

        #[test]
        fn should_support_arithmetic_expression_in_amount() {
            use indoc::indoc;
            let directive = parse(
                indoc! {r#"
                            1970-01-01 "Payee" "Narration"
                              Assets:Bank -(120/10) + 1000 * (25--2) CNY
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::Transaction(..)));
            if let Directive::Transaction(inner) = directive {
                assert_eq!(inner.postings.first().unwrap().to_owned().units.unwrap().number, BigDecimal::from(26988));
            }
        }
    }
    mod budget {
        use bigdecimal::{BigDecimal, One};
        use indoc::indoc;
        use zhang_ast::amount::Amount;
        use zhang_ast::Directive;

        use crate::parser::parse;

        #[test]
        fn should_parse_budget_without_meta() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 custom budget Diet CNY
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::Budget(..)));
            if let Directive::Budget(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.commodity, "CNY");
            }
        }

        #[test]
        fn should_parse_budget_with_meta() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 custom budget Diet CNY
                              alias: "日常饮食"
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::Budget(..)));
            if let Directive::Budget(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.commodity, "CNY");
                assert_eq!(inner.meta.get_one("alias").unwrap().clone().to_plain_string(), "日常饮食");
            }
        }

        #[test]
        fn should_parse_budget_add() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 custom budget-add Diet 1 CNY
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::BudgetAdd(..)));
            if let Directive::BudgetAdd(inner) = directive {
                assert_eq!(inner.name, "Diet");
                assert_eq!(inner.amount, Amount::new(BigDecimal::one(), "CNY".to_owned()));
            }
        }
        #[test]
        fn should_parse_budget_transfer() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 custom budget-transfer Diet Saving 1 CNY
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
            assert!(matches!(directive, Directive::BudgetTransfer(..)));
            if let Directive::BudgetTransfer(inner) = directive {
                assert_eq!(inner.from, "Diet");
                assert_eq!(inner.to, "Saving");
                assert_eq!(inner.amount, Amount::new(BigDecimal::one(), "CNY".to_owned()));
            }
        }

        #[test]
        fn should_parse_budget_close() {
            let directive = parse(
                indoc! {r#"
                            1970-01-01 custom budget-close Diet
                        "#},
                None,
            )
            .unwrap()
            .pop()
            .unwrap()
            .data
            .left()
            .unwrap();
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

            use crate::parser::parse;

            #[test]
            fn should_parse() {
                let directive = parse(
                    indoc! {r#"
                            option "title" "Accounting"
                        "#},
                    None,
                )
                .unwrap()
                .pop()
                .unwrap()
                .data
                .left()
                .unwrap();
                assert!(matches!(directive, Directive::Option(..)));
                if let Directive::Option(inner) = directive {
                    assert_eq!(inner.key.as_str(), "title");
                    assert_eq!(inner.value.as_str(), "Accounting");
                }
            }

            #[test]
            fn should_parse_with_comment() {
                let directive = parse(
                    indoc! {r#"
                            option "title" "Accounting" ;123
                        "#},
                    None,
                )
                .unwrap()
                .pop()
                .unwrap()
                .data
                .left()
                .unwrap();
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

            use crate::parser::parse;

            #[test]
            fn should_parse_with_booking_method() {
                let directive = parse(
                    indoc! {r#"
                            1970-01-01 open Assets:Card CNY       "NONE"
                        "#},
                    None,
                )
                .unwrap()
                .pop()
                .unwrap()
                .data
                .left()
                .unwrap();
                assert!(matches!(directive, Directive::Open(..)));
                if let Directive::Open(inner) = directive {
                    assert_eq!(inner.meta.get_one("booking_method").unwrap().as_str(), "NONE");
                }
            }
        }
    }
}
