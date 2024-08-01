use std::path::PathBuf;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::{NaiveDate, NaiveDateTime};
use itertools::Itertools;
use once_cell::sync::OnceCell;
use pest::iterators::Pairs;
use pest::pratt_parser::PrattParser;
use pest_consume::{match_nodes, Error, Parser};
use snailquote::unescape;
use zhang_ast::amount::Amount;
use zhang_ast::utils::multi_value_map::MultiValueMap;
use zhang_ast::*;

type Result<T> = std::result::Result<T, Error<Rule>>;
type Node<'i> = pest_consume::Node<'i, Rule, ()>;

#[derive(Parser)]
#[grammar = "data_type/text/zhang.pest"]
pub struct ZhangParser;

/// Construct a global [PrattParser] to handle number expressions.
fn pratt_number_parser() -> &'static PrattParser<Rule> {
    static PARSER: OnceCell<PrattParser<Rule>> = OnceCell::new();
    PARSER.get_or_init(|| {
        use pest::pratt_parser::Assoc::*;
        use pest::pratt_parser::Op;
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
            Rule::number => ZhangParser::number(Node::new(primary)),
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
impl ZhangParser {
    #[allow(dead_code)]
    fn EOI(_input: Node) -> Result<()> {
        Ok(())
    }
    fn number_expr(input: Node) -> Result<BigDecimal> {
        parse_number_expr(input.into_pair().into_inner())
    }

    fn number(input: Node) -> Result<BigDecimal> {
        let pure_number = input.as_str().replace([',', '_'], "");
        Ok(BigDecimal::from_str(&pure_number).expect("invalid number detect"))
    }

    fn quote_string(input: Node) -> Result<ZhangString> {
        let string = input.as_str();
        Ok(ZhangString::QuoteString(unescape(string).expect("string contains invalid escape char")))
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
            account_type: AccountType::from_str(&r.0).expect("invalid account type"),
            content: format!("{}:{}", &r.0, r.1.join(":")),
            components: r.1,
        })
    }
    fn date(input: Node) -> Result<Date> {
        let datetime: Date = match_nodes!(input.into_children();
            [date_only(d)] => d,
            [datetime(d)] => d,
            [date_hour(d)] => d
        );
        Ok(datetime)
    }

    fn date_only(input: Node) -> Result<Date> {
        let date = NaiveDate::parse_from_str(input.as_str(), "%Y-%m-%d").expect("cannot construct naive date");
        Ok(Date::Date(date))
    }
    fn datetime(input: Node) -> Result<Date> {
        Ok(Date::Datetime(
            NaiveDateTime::parse_from_str(input.as_str(), "%Y-%m-%d %H:%M:%S").expect("cannot construct naive datetime"),
        ))
    }
    fn date_hour(input: Node) -> Result<Date> {
        Ok(Date::DateHour(
            NaiveDateTime::parse_from_str(input.as_str(), "%Y-%m-%d %H:%M").expect("cannot construct naive date hour"),
        ))
    }

    fn plugin(input: Node) -> Result<Directive> {
        let ret: (ZhangString, Vec<ZhangString>) = match_nodes!(input.into_children();
            [string(module), string(values)..] => (module, values.collect()),
        );
        Ok(Directive::Plugin(Plugin {
            module: ret.0,
            value: ret.1,
            meta: Meta::default(),
        }))
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
    fn valuable_comment(input: Node) -> Result<String> {
        let content: String = match_nodes!(input.into_children();
            [comment_prefix(_), comment_value(v)] => v,
        );
        Ok(content)
    }

    fn comment_prefix(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn comment_value(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }

    fn open(input: Node) -> Result<Directive> {
        let ret: (Date, Account, Vec<String>, Meta) = match_nodes!(input.into_children();
            [date(date), account_name(a), commodity_name(commodities).., metas(metas)] => (date, a, commodities.collect(), metas),
            [date(date), account_name(a), commodity_name(commodities)..] => (date, a, commodities.collect(), Meta::default()),
            [date(date), account_name(a), metas(metas)] => (date, a, vec![], metas),
        );

        let open = Open {
            date: ret.0,
            account: ret.1,
            commodities: ret.2,
            meta: ret.3,
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
    fn indentation(input: Node) -> Result<()> {
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

    fn posting_unit(input: Node) -> Result<(Option<Amount>, Option<(Option<PostingCost>, Option<SingleTotalPrice>)>)> {
        let ret: (Option<Amount>, Option<(Option<PostingCost>, Option<SingleTotalPrice>)>) = match_nodes!(input.into_children();
            [posting_amount(amount)] => (Some(amount), None),
            [posting_meta(meta)] => (None, Some(meta)),
            [posting_amount(amount), posting_meta(meta)] => (Some(amount), Some(meta)),
        );
        Ok(ret)
    }

    fn posting_cost_prefix(input: Node) -> Result<()> {
        Ok(())
    }
    fn posting_cost_postfix(input: Node) -> Result<()> {
        Ok(())
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
        Ok(Some(Flag::from_str(input.as_str().trim()).expect("cannot read node as str")))
    }

    fn posting_price(input: Node) -> Result<SingleTotalPrice> {
        let ret: SingleTotalPrice = match_nodes!(input.into_children();
            [posting_total_price(p)] => SingleTotalPrice::Total(p),
            [posting_single_price(p)] => SingleTotalPrice::Single(p),
        );
        Ok(ret)
    }
    fn posting_meta(input: Node) -> Result<(Option<PostingCost>, Option<SingleTotalPrice>)> {
        let ret: (Option<PostingCost>, Option<SingleTotalPrice>) = match_nodes!(input.into_children();
            [] => (None, None),
            [posting_cost_prefix(_), posting_cost_postfix(_)] => (Some(PostingCost{base: None,date: None}), None),
            [posting_cost_prefix(_), posting_cost(cost), posting_cost_postfix(_)] => (Some(PostingCost{base: Some(cost),date: None}), None),
            [posting_price(p)] => (None, Some(p)),
            [posting_cost_prefix(_), posting_cost_postfix(_), posting_price(p)] => (Some(PostingCost{base: None,date: None}), Some(p)),
            [posting_cost_prefix(_), posting_cost(cost), date(d), posting_cost_postfix(_)] => (Some(PostingCost{base: Some(cost),date: Some(d)}) , None),
            [posting_cost_prefix(_), posting_cost(cost), posting_cost_postfix(_),  posting_price(p)] => (Some(PostingCost{base: Some(cost),date: None}), Some(p)),
            [posting_cost_prefix(_), posting_cost(cost), date(d), posting_cost_postfix(_), posting_price(p)] => (Some(PostingCost{base: Some(cost),date: Some(d)}), Some(p)),
        );
        Ok(ret)
    }
    fn transaction_posting(input: Node) -> Result<Posting> {
        let ret: (
            Option<Flag>,
            Account,
            Option<(Option<Amount>, Option<(Option<PostingCost>, Option<SingleTotalPrice>)>)>,
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
            price: None,
            comment: None,
            meta,
        };

        if let Some((amount, meta)) = unit {
            line.units = amount;

            if let Some((posting_cost, price)) = meta {
                line.cost = posting_cost;
                line.price = price;
            }
        }
        Ok(line)
    }

    fn transaction_line(input: Node) -> Result<(Option<Posting>, Option<(String, ZhangString)>)> {
        let ret: (Option<Posting>, Option<(String, ZhangString)>) = match_nodes!(input.into_children();
            [transaction_posting(posting)] => (Some(posting), None),
            [transaction_posting(posting), valuable_comment(comment)] => (Some(posting.set_comment(comment)), None),
            [key_value_line(meta)] => (None, Some(meta)),
            [key_value_line(meta), valuable_comment(_)] => (None, Some(meta)),

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
        let ret: (Date, ZhangString, Vec<StringOrAccount>, Meta) = match_nodes!(input.into_children();
            [date(date), string(module), string_or_account(options)..] => (date, module, options.collect(), Meta::default()),
            [date(date), string(module), string_or_account(options).., metas(metas)] => (date, module, options.collect(), metas),
        );
        Ok(Directive::Custom(Custom {
            date: ret.0,
            custom_type: ret.1,
            values: ret.2,
            meta: ret.3,
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

    fn balance(input: Node) -> Result<Directive> {
        let ret: (Date, Account, BigDecimal, String, Option<Account>) = match_nodes!(input.into_children();
            [date(date), account_name(name), number_expr(amount), commodity_name(commodity)] => (date, name, amount, commodity, None),
            [date(date), account_name(name), number_expr(amount), commodity_name(commodity), account_name(pad)] => (date, name, amount, commodity, Some(pad)),
        );
        if let Some(pad) = ret.4 {
            Ok(Directive::BalancePad(BalancePad {
                date: ret.0,
                account: ret.1,
                amount: Amount::new(ret.2, ret.3),
                pad,
                meta: Default::default(),
            }))
        } else {
            Ok(Directive::BalanceCheck(BalanceCheck {
                date: ret.0,
                account: ret.1,
                amount: Amount::new(ret.2, ret.3),
                meta: Default::default(),
            }))
        }
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
            [date(date), unquote_string(from), unquote_string(to), posting_amount(amount)] => (date, from, to, amount,Meta::default()),
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

    fn metable_head(input: Node) -> Result<Directive> {
        let ret: Directive = match_nodes!(input.into_children();
            [open(item)] => item,
            [close(item)] => item,
            [note(item)] => item,
            [event(item)] => item,
            [document(item)] => item,
            [balance(item)] => item,
            [price(item)] => item,
            [commodity(item)] => item,
            [custom(item)] => item,
            [comment(item)] => item,
            [budget(item)] => item,
            [budget_close(item)] => item,
            [budget_add(item)] => item,
            [budget_transfer(item)] => item,
            [plugin(item)] => item,
        );
        Ok(ret)
    }
    fn empty_space_line(input: Node) -> Result<()> {
        Ok(())
    }
    fn item(input: Node) -> Result<Option<(Directive, SpanInfo)>> {
        let span = input.as_span();
        let span_info = SpanInfo {
            start: span.start_pos().pos(),
            end: span.end_pos().pos(),
            content: span.as_str().to_string(),
            filename: None,
        };
        let ret: Option<Directive> = match_nodes!(input.into_children();
            [option(item)] => Some(item),
            [include(item)] => Some(item),
            [valuable_comment(item)] => Some(Directive::Comment(Comment { content:item })),

            [transaction(item)] => Some(item),
            [empty_space_line(_)] => None,
            [metable_head(head)] => Some(head),
            [metable_head(head), metas(meta)] => {
                Some(head.set_meta(meta))
            },
        );
        Ok(ret.map(|it| (it, span_info)))
    }
    fn entry(input: Node) -> Result<Vec<Spanned<Directive>>> {
        let ret: Vec<(Directive, SpanInfo)> = match_nodes!(input.into_children();
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

pub fn parse(input_str: &str, file: impl Into<Option<PathBuf>>) -> Result<Vec<Spanned<Directive>>> {
    let file = file.into();
    let inputs = ZhangParser::parse(Rule::entry, input_str)?;
    let input = inputs.single()?;
    ZhangParser::entry(input).map(|mut directives| {
        directives.iter_mut().for_each(|directive| directive.span.filename.clone_from(&file));
        directives
    })
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
                        date: None
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
                        date: None
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
                assert_eq!(Some(PostingCost { base: None, date: None }), posting.cost);
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
                assert_eq!(Some(PostingCost { base: None, date: None }), posting.cost);
                assert_eq!(Some(SingleTotalPrice::Single(Amount::new(BigDecimal::from(7i32), "CNY"))), posting.price);
            }
            #[test]
            fn should_parse_metas_in_posting() {
                let mut trx = get_first_posting(indoc! {r#"
                2022-06-02 "balanced transaction"
                  Assets:Card -100 USD
                    a: b
                "#});
                let posting = trx.postings.pop().unwrap();
                assert_eq!("b", posting.meta.get_one("a").cloned().unwrap().to_plain_string());
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
