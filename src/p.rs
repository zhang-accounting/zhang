use std::str::FromStr;

use bigdecimal::BigDecimal;
use chrono::NaiveDate;
use pest_consume::{Error, match_nodes, Parser};
use snailquote::unescape;

use crate::account::AccountType;
use crate::models::{
    Account, Amount, AvaroString, Directive, Flag, Price, StringOrAccount,
    Transaction, TransactionLine,
};

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
    fn QuoteString(input: Node) -> Result<AvaroString> {
        let string = input.as_str();
        Ok(AvaroString::QuoteString(unescape(string).unwrap()))
    }

    fn UnquoteString(input: Node) -> Result<AvaroString> {
        Ok(AvaroString::UnquoteString(input.as_str().to_owned()))
    }

    fn String(input: Node) -> Result<AvaroString> {
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
        let r: (String, Vec<AvaroString>) = match_nodes!(input.into_children();
            [AccountType(a), UnquoteString(i)..] => {
                (a, i.collect())
            },

        );
        Ok(Account {
            account_type: AccountType::from_str(&r.0).unwrap(),
            value: r.1.into_iter().map(|it| it.to_string()).collect(),
        })
    }
    fn Date(input: Node) -> Result<NaiveDate> {
        Ok(NaiveDate::parse_from_str(input.as_str(), "%Y-%m-%d").unwrap())
    }

    fn Plugin(input: Node) -> Result<Directive> {
        let ret: (AvaroString, Vec<AvaroString>) = match_nodes!(input.into_children();
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
        let ret: (NaiveDate, Account, Vec<String>, Vec<(AvaroString, AvaroString)>) = match_nodes!(input.into_children();
            [Date(date), AccountName(a), CommodityName(commodities).., CommodityMeta(metas)] => (date, a, commodities.collect(), metas),
            [Date(date), AccountName(a), CommodityName(commodities)..] => (date, a, commodities.collect(), vec![]),
            [Date(date), AccountName(a), CommodityMeta(metas)] => (date, a, vec![], metas),
        );
        Ok(Directive::Open {
            date: ret.0,
            account: ret.1,
            commodities: ret.2,
        })
    }
    fn Close(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account) = match_nodes!(input.into_children();
            [Date(date), AccountName(a)] => (date, a)
        );
        Ok(Directive::Close {
            date: ret.0,
            account: ret.1,
        })
    }

    fn identation(input: Node) -> Result<()> {
        Ok(())
    }

    fn CommodityLine(input: Node) -> Result<(AvaroString, AvaroString)> {
        let ret: (AvaroString, AvaroString) = match_nodes!(input.into_children();
            [String(key), String(value)] => (key, value),
        );
        Ok(ret)
    }

    fn CommodityMeta(input: Node) -> Result<Vec<(AvaroString, AvaroString)>> {
        let ret: Vec<(AvaroString, AvaroString)> = match_nodes!(input.into_children();
            [CommodityLine(lines)..] => lines.collect(),
        );
        Ok(ret)
    }

    fn PostingCost(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => (amount,c),
        );
        Ok(ret)
    }
    fn PostingTotalPrice(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => (amount,c),
        );
        Ok(ret)
    }
    fn PostingSinglePrice(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => (amount,c),
        );
        Ok(ret)
    }
    fn PostingAmount(input: Node) -> Result<Amount> {
        let ret: Amount = match_nodes!(input.into_children();
            [number(amount), CommodityName(c)] => (amount,c),
        );
        Ok(ret)
    }

    fn TransactionFlag(input: Node) -> Result<Option<Flag>> {
        Ok(Some(Flag::from_str(input.as_str().trim()).unwrap()))
    }

    fn PostingPrice(input: Node) -> Result<Price> {
        let ret: Price = match_nodes!(input.into_children();
            [PostingTotalPrice(p)] => Price::Total(p),
            [PostingSinglePrice(p)] => Price::Single(p),
        );
        Ok(ret)
    }
    fn PostingMeta(input: Node) -> Result<(Option<Amount>, Option<NaiveDate>, Option<Price>)> {
        let ret: (Option<Amount>, Option<NaiveDate>, Option<Price>) = match_nodes!(input.into_children();
            [] => (None, None, None),
            [PostingCost(cost)] => (Some(cost), None, None),
            [PostingPrice(p)] => (None, None, Some(p)),
            [PostingCost(cost), Date(d)] => (Some(cost), Some(d), None),
            [PostingCost(cost), PostingPrice(p)] => (Some(cost), None, Some(p)),
            [PostingCost(cost), Date(d), PostingPrice(p)] => (Some(cost), Some(d), Some(p)),
        );
        Ok(ret)
    }
    fn TransactionPosting(input: Node) -> Result<TransactionLine> {
        let ret: (
            Option<Flag>,
            Account,
            Option<Amount>,
            Option<(Option<Amount>, Option<NaiveDate>, Option<Price>)>,
        ) = match_nodes!(input.into_children();
            [AccountName(account_name)] => (None, account_name, None, None),
            [AccountName(account_name), PostingAmount(amount)] => (None, account_name, Some(amount), None),
            [AccountName(account_name),  PostingMeta(meta)] => (None, account_name, None, Some(meta)),
            [AccountName(account_name), PostingAmount(amount), PostingMeta(meta)] => (None, account_name, Some(amount), Some(meta)),
            [TransactionFlag(flag), AccountName(account_name)] => (flag, account_name, None, None),
            [TransactionFlag(flag), AccountName(account_name), PostingAmount(amount)] => (flag, account_name, Some(amount), None),
            [TransactionFlag(flag), AccountName(account_name), PostingAmount(amount), PostingMeta(meta)] => (flag, account_name, Some(amount), Some(meta)),
        );
        let (_, _, _, meta) = ret;

        let mut line = TransactionLine {
            flag: ret.0,
            account: ret.1,
            amount: ret.2,
            cost: None,
            cost_date: None,
            price: None,
        };
        if let Some(meta) = meta {
            line.cost = meta.0;
            line.cost_date = meta.1;
            line.price = meta.2;
        }
        Ok(line)
    }

    fn TransactionLine(
        input: Node,
    ) -> Result<(Option<TransactionLine>, Option<(AvaroString, AvaroString)>)> {
        let ret: (Option<TransactionLine>, Option<(AvaroString, AvaroString)>) = match_nodes!(input.into_children();
            [TransactionPosting(posting)] => (Some(posting), None),
            [CommodityLine(meta)] => (None, Some(meta)),

        );
        Ok(ret)
    }
    fn TransactionLines(
        input: Node,
    ) -> Result<Vec<(Option<TransactionLine>, Option<(AvaroString, AvaroString)>)>> {
        let ret = match_nodes!(input.into_children();
            [TransactionLine(lines)..] => lines.collect(),
        );
        Ok(ret)
    }

    fn Tag(input: Node) -> Result<AvaroString> {
        let ret = match_nodes!(input.into_children();
            [UnquoteString(tag)] => tag,
        );
        Ok(ret)
    }
    fn Link(input: Node) -> Result<AvaroString> {
        let ret = match_nodes!(input.into_children();
            [UnquoteString(tag)] => tag,
        );
        Ok(ret)
    }
    fn Tags(input: Node) -> Result<Vec<AvaroString>> {
        let ret = match_nodes!(input.into_children();
            [Tag(tags)..] => tags.collect(),
        );
        Ok(ret)
    }
    fn Links(input: Node) -> Result<Vec<AvaroString>> {
        let ret = match_nodes!(input.into_children();
            [Link(links)..] => links.collect(),
        );
        Ok(ret)
    }

    fn Transaction(input: Node) -> Result<Directive> {
        let ret: (
            NaiveDate,
            Option<Flag>,
            Option<AvaroString>,
            Option<AvaroString>,
            Vec<AvaroString>,
            Vec<AvaroString>,
            Vec<(Option<TransactionLine>, Option<(AvaroString, AvaroString)>)>,
        ) = match_nodes!(input.into_children();
            [Date(date), TransactionFlag(flag), Tags(tags), Links(links), TransactionLines(lines)] => (date, flag, None, None, tags, links, lines),
            [Date(date), TransactionFlag(flag), QuoteString(narration), Tags(tags), Links(links), TransactionLines(lines)] => (date, flag, None, Some(narration), tags, links, lines),
            [Date(date), TransactionFlag(flag), QuoteString(payee), QuoteString(narration), Tags(tags), Links(links), TransactionLines(lines)] => (date, flag, Some(payee), Some(narration), tags, links,lines),
        );
        let mut transaction = Transaction {
            date: ret.0,
            flag: ret.1,
            payee: ret.2,
            narration: ret.3,
            tags: ret.4,
            links: ret.5,
            lines: vec![],
            metas: vec![],
        };

        for line in ret.6 {
            match line {
                (Some(trx), None) => transaction.lines.push(trx),
                (None, Some(meta)) => transaction.metas.push(meta),
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
        Ok(Directive::Commodity {
            date: ret.0,
            name: ret.1,
            metas: ret.2,
        })
    }

    fn StringOrAccount(input: Node) -> Result<StringOrAccount> {
        let ret: StringOrAccount = match_nodes!(input.into_children();
            [String(value)] => StringOrAccount::String(value),
            [AccountName(value)] => StringOrAccount::Account(value),
        );
        Ok(ret)
    }

    fn Custom(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, AvaroString, Vec<StringOrAccount>) = match_nodes!(input.into_children();
            [Date(date), String(module), StringOrAccount(options)..] => (date, module, options.collect()),
        );
        Ok(Directive::Custom {
            date: ret.0,
            type_name: ret.1,
            values: ret.2,
        })
    }

    fn Include(input: Node) -> Result<Directive> {
        let ret: AvaroString = match_nodes!(input.into_children();
            [QuoteString(path)] => path,
        );
        Ok(Directive::Include {
            file: ret.to_string(),
        })
    }

    fn Note(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, AvaroString) = match_nodes!(input.into_children();
            [Date(date), AccountName(a), String(path)] => (date, a, path),
        );
        Ok(Directive::Note {
            date: ret.0,
            account: ret.1,
            description: ret.2.to_string(),
        })
    }

    fn Pad(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, Account) = match_nodes!(input.into_children();
            [Date(date), AccountName(from), AccountName(to)] => (date, from, to),
        );
        Ok(Directive::Pad {
            date: ret.0,
            from: ret.1,
            to: ret.2,
        })
    }

    fn Event(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, AvaroString, AvaroString) = match_nodes!(input.into_children();
            [Date(date), String(name), String(value)] => (date, name, value),
        );
        Ok(Directive::Event {
            date: ret.0,
            name: ret.1.to_string(),
            value: ret.2.to_string(),
        })
    }

    fn Balance(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, BigDecimal, String) = match_nodes!(input.into_children();
            [Date(date), AccountName(name), number(amount), CommodityName(commodity)] => (date, name, amount, commodity),
        );
        Ok(Directive::Balance {
            date: ret.0,
            account: ret.1,
            amount: (ret.2, ret.3),
        })
    }

    fn Document(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, AvaroString) = match_nodes!(input.into_children();
            [Date(date), AccountName(name), String(path)] => (date, name, path),
        );
        Ok(Directive::Document {
            date: ret.0,
            account: ret.1,
            path: ret.2.to_string(),
        })
    }

    fn Price(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, String, BigDecimal, String) = match_nodes!(input.into_children();
            [Date(date), CommodityName(source), number(price), CommodityName(target)] => (date, source, price, target)
        );
        Ok(Directive::Price {
            date: ret.0,
            commodity: ret.1,
            amount: (ret.2, ret.3),
        })
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
