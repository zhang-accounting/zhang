use pest_consume::{Parser, Error, match_nodes};
use bigdecimal::BigDecimal;
use std::str::FromStr;
use crate::models::{AvaroString, AccountType, Directive, Account};
use chrono::NaiveDate;

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
        let ret: String = match_nodes!(
            input.into_children();
            [inner(i)] => i
        );
        Ok(AvaroString::QuoteString(ret))
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
        Ok(Directive::Plugin { module: ret.0.to_string(), value: values })
    }

    fn Option(input: Node) -> Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [String(key), String(value)] => Directive::Option {key:key.to_string(),value:value.to_string()},
        );
        Ok(ret)
    }

    fn Open(input: Node) -> Result<Directive> {
        let ret: (NaiveDate, Account, Vec<String>) = match_nodes!(input.into_children();
            [Date(date), AccountName(a), CommodityName(commodities)..] => (date, a, commodities.collect())
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
    fn Commodity(input: Node) -> Result<Directive> {
        todo!()
    }
    fn Include(input: Node) -> Result<Directive> {
        let ret: AvaroString = match_nodes!(input.into_children();
            [QuoteString(path)] => path,
        );
        Ok(Directive::Include { file: ret.to_string() })
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
            path: ret.2.to_string()
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
    // Parse the input into `Nodes`
    let inputs = AvaroParser::parse(Rule::Entry, input_str)?;
    // There should be a single root node in the parsed tree
    let input = inputs.single()?;
    // Consume the `Node` recursively into the final value
    AvaroParser::Entry(input)
}