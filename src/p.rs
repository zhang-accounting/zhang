use std::collections::HashMap;
use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use pest_consume::{match_nodes, Error, Parser};
use snailquote::unescape;

use crate::account::{Account, AccountType};
use crate::amount::Amount;
use crate::data::{Balance, Close, Commodity, Custom, Document, Event, Note, Open, Pad, Posting, Price, Transaction};
use crate::models::{Directive, Flag, SingleTotalPrice, StringOrAccount};

type Result<T> = std::result::Result<T, Error<Rule>>;
type Node<'i> = pest_consume::Node<'i, Rule, ()>;

#[derive(Parser)]
#[grammar = "avaro.pest"]
pub struct AvaroParser;

#[pest_consume::parser]
impl AvaroParser {
    fn EOI(_input: Node) -> Result<()> {
        Ok(())
    }
    fn number(input: Node) -> Result<BigDecimal> {
        Ok(BigDecimal::from_str(input.as_str()).unwrap())
    }
    fn inner(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn QuoteString(input: Node) -> Result<String> {
        let string = input.as_str();
        Ok(unescape(string).unwrap())
    }

    fn UnquoteString(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }

    fn String(input: Node) -> Result<String> {
        let ret = match_nodes!(
            input.into_children();
            [QuoteString(i)] => i,
            [UnquoteString(i)] => i
        );
        Ok(ret)
    }
    fn CommodityName(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn AccountType(input: Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }
    fn AccountName(input: Node) -> Result<Account> {
        let r: (String, Vec<String>) = match_nodes!(input.into_children();
            [AccountType(a), UnquoteString(i)..] => {
                (a, i.collect())
            },

        );
        Ok(Account {
            account_type: AccountType::from_str(&r.0).unwrap(),
            content: format!("{}:{}", &r.0, r.1.join(":")),
            components: r.1,
        })
    }
    fn Date(input: Node) -> Result<NaiveDate> {
        Ok(NaiveDate::parse_from_str(input.as_str(), "%Y-%m-%d").unwrap())
    }

    fn Plugin(input: Node) -> Result<Directive> {
        let ret: (String, Vec<String>) = match_nodes!(input.into_children();
            [String(module), String(values)..] => (module, values.collect()),
        );
        let values = ret.1.into_iter().map(|it| it.to_string()).collect();
        Ok(Directive::Plugin {
            module: ret.0.to_string(),
            value: values,
        })
    }

    fn Option(input: Node) -> Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [String(key), String(value)] => Directive::Option {key:key.to_string(),value:value.to_string()},
        );
        Ok(ret)
    }
    fn Comment(input: Node) -> Result<Directive> {
        Ok(Directive::Comment {
            content: input.as_str().to_owned(),
        })
    }

    fn Open(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, Vec<String>, Vec<(String, String)>) = match_nodes!(input.into_children();
            [Date(date), AccountName(a), CommodityName(commodities).., CommodityMeta(metas)] => (date, a, commodities.collect(), metas),
            [Date(date), AccountName(a), CommodityName(commodities)..] => (date, a, commodities.collect(), vec![]),
            [Date(date), AccountName(a), CommodityMeta(metas)] => (date, a, vec![], metas),
        );
        let open = Open {
            date: ret.0.and_hms(0, 0, 0),
            account: ret.1,
            commodities: ret.2,
            meta: ret.3.into_iter().collect(),
        };
        Ok(Directive::Open(open))
    }
    fn Close(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account) = match_nodes!(input.into_children();
            [Date(date), AccountName(a)] => (date, a)
        );
        Ok(Directive::Close(Close {
            date: ret.0.and_hms(0, 0, 0),
            account: ret.1,
            meta: Default::default(),
        }))
    }

    fn identation(input: Node) -> Result<()> {
        Ok(())
    }

    fn CommodityLine(input: Node) -> Result<(String, String)> {
        let ret: (String, String) = match_nodes!(input.into_children();
            [String(key), String(value)] => (key, value),
        );
        Ok(ret)
    }

    fn CommodityMeta(input: Node) -> Result<Vec<(String, String)>> {
        let ret: Vec<(String, String)> = match_nodes!(input.into_children();
            [CommodityLine(lines)..] => lines.collect(),
        );
        Ok(ret)
    }

    fn PostingUnit(
        input: Node,
    ) -> Result<(
        Amount,
        Option<(Option<Amount>, Option<NaiveDate>, Option<SingleTotalPrice>)>,
    )> {
        let ret: (
            Amount,
            Option<(Option<Amount>, Option<NaiveDate>, Option<SingleTotalPrice>)>,
        ) = match_nodes!(input.into_children();
            [PostingAmount(amount)] => (amount, None),
            [PostingAmount(amount), PostingMeta(meta)] => (amount, Some(meta)),
        );
        Ok(ret)
    }

    fn PostingCost(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }
    fn PostingTotalPrice(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }
    fn PostingSinglePrice(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }

    fn PostingAmount(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => Amount::new(amount, c),
        );
        Ok(ret)
    }

    fn TransactionFlag(input: Node) -> Result<Option<Flag>> {
        Ok(Some(Flag::from_str(input.as_str().trim()).unwrap()))
    }

    fn PostingPrice(input: Node) -> Result<SingleTotalPrice> {
        let ret: SingleTotalPrice = match_nodes!(input.into_children();
            [PostingTotalPrice(p)] => SingleTotalPrice::Total(p),
            [PostingSinglePrice(p)] => SingleTotalPrice::Single(p),
        );
        Ok(ret)
    }
    fn PostingMeta(input: Node) -> Result<(Option<Amount>, Option<NaiveDate>, Option<SingleTotalPrice>)> {
        let ret: (Option<Amount>, Option<NaiveDate>, Option<SingleTotalPrice>) = match_nodes!(input.into_children();
            [] => (None, None, None),
            [PostingCost(cost)] => (Some(cost), None, None),
            [PostingPrice(p)] => (None, None, Some(p)),
            [PostingCost(cost), Date(d)] => (Some(cost), Some(d), None),
            [PostingCost(cost), PostingPrice(p)] => (Some(cost), None, Some(p)),
            [PostingCost(cost), Date(d), PostingPrice(p)] => (Some(cost), Some(d), Some(p)),
        );
        Ok(ret)
    }
    fn TransactionPosting(input: Node) -> Result<Posting> {
        let ret: (
            Option<Flag>,
            Account,
            Option<(
                Amount,
                Option<(Option<Amount>, Option<NaiveDate>, Option<SingleTotalPrice>)>,
            )>,
        ) = match_nodes!(input.into_children();
            [AccountName(account_name)] => (None, account_name, None),
            [AccountName(account_name), PostingUnit(unit)] => (None, account_name, Some(unit)),
            [TransactionFlag(flag), AccountName(account_name)] => (flag, account_name, None),
            [TransactionFlag(flag), AccountName(account_name), PostingUnit(unit)] => (flag, account_name, Some(unit)),
        );

        let (flag, account, unit) = ret;

        let mut line = Posting {
            flag,
            account,
            units: None,
            cost: None,
            price: None,
            meta: Default::default(),
        };

        if let Some((amount, meta)) = unit {
            line.units = Some(amount);

            if let Some(meta) = meta {
                line.cost = meta.0;
                // line.price = meta.2; // todo
            }
        }
        Ok(line)
    }

    fn TransactionLine(input: Node) -> Result<(Option<Posting>, Option<(String, String)>)> {
        let ret: (Option<Posting>, Option<(String, String)>) = match_nodes!(input.into_children();
            [TransactionPosting(posting)] => (Some(posting), None),
            [CommodityLine(meta)] => (None, Some(meta)),

        );
        Ok(ret)
    }
    fn TransactionLines(
        input: Node,
    ) -> Result<Vec<(Option<Posting>, Option<(String, String)>)>> {
        let ret = match_nodes!(input.into_children();
            [TransactionLine(lines)..] => lines.collect(),
        );
        Ok(ret)
    }

    fn Tag(input: Node) -> Result<String> {
        let ret = match_nodes!(input.into_children();
            [UnquoteString(tag)] => tag,
        );
        Ok(ret)
    }
    fn Link(input: Node) -> Result<String> {
        let ret = match_nodes!(input.into_children();
            [UnquoteString(tag)] => tag,
        );
        Ok(ret)
    }
    fn Tags(input: Node) -> Result<Vec<String>> {
        let ret = match_nodes!(input.into_children();
            [Tag(tags)..] => tags.collect(),
        );
        Ok(ret)
    }
    fn Links(input: Node) -> Result<Vec<String>> {
        let ret = match_nodes!(input.into_children();
            [Link(links)..] => links.collect(),
        );
        Ok(ret)
    }

    fn Transaction(input: Node) -> Result<Directive> {
        let ret: (
            NaiveDate,
            Option<Flag>,
            Option<String>,
            Option<String>,
            Vec<String>,
            Vec<String>,
            Vec<(Option<Posting>, Option<(String, String)>)>,
        ) = match_nodes!(input.into_children();
            [Date(date), TransactionFlag(flag), Tags(tags), Links(links), TransactionLines(lines)] => (date, flag, None, None, tags, links, lines),
            [Date(date), TransactionFlag(flag), QuoteString(narration), Tags(tags), Links(links), TransactionLines(lines)] => (date, flag, None, Some(narration), tags, links, lines),
            [Date(date), TransactionFlag(flag), QuoteString(payee), QuoteString(narration), Tags(tags), Links(links), TransactionLines(lines)] => (date, flag, Some(payee), Some(narration), tags, links,lines),
        );
        let mut transaction = Transaction {
            date: ret.0.and_hms(0, 0, 0),
            flag: ret.1,
            payee: ret.2,
            narration: ret.3,
            tags: ret.4.into_iter().collect(),
            links: ret.5.into_iter().collect(),
            postings: vec![],
            meta: HashMap::default(),
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

    fn Commodity(input: Node) -> Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [Date(date), CommodityName(name)] => (date, name, vec![]),
            [Date(date), CommodityName(name), CommodityMeta(meta)] => (date, name, meta),
        );
        Ok(Directive::Commodity(Commodity{
            date: ret.0.and_hms(0,0,0),
            currency: ret.1,
            meta: ret.2.into_iter().collect()
        }))
    }

    fn StringOrAccount(input: Node) -> Result<StringOrAccount> {
        let ret: StringOrAccount = match_nodes!(input.into_children();
            [String(value)] => StringOrAccount::String(value),
            [AccountName(value)] => StringOrAccount::Account(value),
        );
        Ok(ret)
    }

    fn Custom(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, String, Vec<StringOrAccount>) = match_nodes!(input.into_children();
            [Date(date), String(module), StringOrAccount(options)..] => (date, module, options.collect()),
        );
        Ok(Directive::Custom(Custom{
            date: ret.0.and_hms(0,0,0),
            custom_type: ret.1,
            values: ret.2,
            meta: Default::default()
        }))
    }

    fn Include(input: Node) -> Result<Directive> {
        let ret: String = match_nodes!(input.into_children();
            [QuoteString(path)] => path,
        );
        Ok(Directive::Include {
            file: ret.to_string(),
        })
    }

    fn Note(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, String) = match_nodes!(input.into_children();
            [Date(date), AccountName(a), String(path)] => (date, a, path),
        );
        Ok(Directive::Note(Note{
            date: ret.0.and_hms(0,0,0),
            account: ret.1,
            comment: ret.2,
            tags: None,
            links: None,
            meta: Default::default()
        }))
    }

    fn Pad(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, Account) = match_nodes!(input.into_children();
            [Date(date), AccountName(from), AccountName(to)] => (date, from, to),
        );
        Ok(Directive::Pad(Pad {
            date: ret.0.and_hms(0,0,0),
            account: ret.1,
            source: ret.2,
            meta: Default::default()
        }))
    }

    fn Event(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, String, String) = match_nodes!(input.into_children();
            [Date(date), String(name), String(value)] => (date, name, value),
        );
        Ok(Directive::Event(Event{
            date: ret.0.and_hms(0,0,0),
            event_type: ret.1,
            description: ret.2,
            meta: Default::default()
        }))
    }

    fn Balance(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, BigDecimal, String) = match_nodes!(input.into_children();
            [Date(date), AccountName(name), number(amount), CommodityName(commodity)] => (date, name, amount, commodity),
        );
        Ok(Directive::Balance(Balance{
            date: ret.0.and_hms(0,0,0),
            account: ret.1,
            amount: Amount::new(ret.2, ret.3),
            tolerance: None,
            diff_amount: None,
            meta: Default::default()
        }))
    }

    fn Document(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, String) = match_nodes!(input.into_children();
            [Date(date), AccountName(name), String(path)] => (date, name, path),
        );
        Ok(Directive::Document(Document{
            date: ret.0.and_hms(0,0,0),
            account: ret.1,
            filename: ret.2,
            tags: None,
            links: None,
            meta: Default::default()
        }))
    }

    fn Price(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, String, BigDecimal, String) = match_nodes!(input.into_children();
            [Date(date), CommodityName(source), number(price), CommodityName(target)] => (date, source, price, target)
        );
        Ok(Directive::Price(Price{
            date: ret.0.and_hms(0,0,0),
            currency: ret.1,
            amount: Amount::new(ret.2, ret.3),
            meta: Default::default()
        }))
    }

    fn Item(input: Node) -> Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [Option(item)] => item,
            [Open(item)] => item,
            [Plugin(item)] => item,
            [Close(item)] => item,
            [Include(item)] => item,
            [Note(item)] => item,
            [Pad(item)] => item,
            [Event(item)] => item,
            [Document(item)] => item,
            [Balance(item)] => item,
            [Price(item)] => item,
            [Commodity(item)] => item,
            [Custom(item)] => item,
            [Comment(item)] => item,
            [Transaction(item)] => item,
        );
        Ok(ret)
    }
    fn Entry(input: Node) -> Result<Vec<Directive>> {
        let ret = match_nodes!(input.into_children();
            [Item(items).., _] => items.collect(),
        );
        Ok(ret)
    }
}

pub fn parse_avaro(input_str: &str) -> Result<Vec<Directive>> {
    let inputs = AvaroParser::parse(Rule::Entry, input_str)?;
    let input = inputs.single()?;
    AvaroParser::Entry(input)
}

pub fn parse_account(input_str: &str) -> Result<Account> {
    let inputs = AvaroParser::parse(Rule::AccountName, input_str)?;
    let input = inputs.single()?;
    AvaroParser::AccountName(input)
}
