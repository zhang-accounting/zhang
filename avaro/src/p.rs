use pest_consume::{Parser, Error, match_nodes};
use bigdecimal::BigDecimal;
use std::str::FromStr;
use crate::models::{AvaroString, AccountType, Directive};

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
    fn QuoteString(input:Node) -> Result<AvaroString> {
        let ret:String = match_nodes!(
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
    fn AccountName(input: Node) -> Result<String> {
        let r = match_nodes!(input.into_children();
            [AccountType(a), UnquoteString(i)..] => {
                let mut ret: Vec<String> = i.into_iter().map(|i|i.to_string()).collect();
                ret.push(a);
                ret
            },

        );
        Ok(r.join(""))
    }
    fn Date(input:Node) -> Result<String> {
        Ok(input.as_str().to_owned())
    }

    fn Option(input:Node) -> Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [String(key), String(value)] => Directive::Option {key:key.to_string(),value:value.to_string()},
        );
        Ok(ret)
    }

    fn Item(input: Node) ->Result<Directive> {
        let ret = match_nodes!(input.into_children();
            [Option(item)] => item,
        );
        Ok(ret)
    }
    fn Entry(input: Node) ->Result<Vec<Directive>> {
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