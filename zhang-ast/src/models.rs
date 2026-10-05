use std::fmt::Display;
use std::str::FromStr;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumDiscriminants, EnumString};

use crate::account::Account;
use crate::amount::Amount;
use crate::data::{Close, Comment, Commodity, Custom, Document, Event, Include, Note, Open, Options, Pad, Plugin, Price, Query, Transaction};
use crate::error::ErrorKind;
use crate::{BalanceCheck, BalancePad, Budget, BudgetAdd, BudgetClose, BudgetTransfer, Date, Meta};

/// [`DirectiveType`] is the kind of a directive, named like its variant
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize, EnumDiscriminants)]
#[strum_discriminants(name(DirectiveType), derive(EnumString, Display))]
pub enum Directive {
    Open(Open),
    Close(Close),
    Commodity(Commodity),
    Transaction(Transaction),
    BalancePad(BalancePad),
    BalanceCheck(BalanceCheck),
    Pad(Pad),
    Note(Note),
    Document(Document),
    Price(Price),
    Event(Event),
    Custom(Custom),
    Query(Query),
    Option(Options),
    Plugin(Plugin),
    Include(Include),
    Comment(Comment),

    Budget(Budget),
    BudgetAdd(BudgetAdd),
    BudgetTransfer(BudgetTransfer),
    BudgetClose(BudgetClose),
}

/// `$body` on the payload `$it` of a directive with a date, `$otherwise` on the others: an option, a plugin, an
/// include and a comment
macro_rules! when_dated {
    ($directive: expr, |$it: ident| $body: expr, $otherwise: expr) => {
        match $directive {
            Directive::Open($it) => $body,
            Directive::Close($it) => $body,
            Directive::Commodity($it) => $body,
            Directive::Transaction($it) => $body,
            Directive::BalancePad($it) => $body,
            Directive::BalanceCheck($it) => $body,
            Directive::Pad($it) => $body,
            Directive::Note($it) => $body,
            Directive::Document($it) => $body,
            Directive::Price($it) => $body,
            Directive::Event($it) => $body,
            Directive::Custom($it) => $body,
            Directive::Query($it) => $body,
            Directive::Budget($it) => $body,
            Directive::BudgetAdd($it) => $body,
            Directive::BudgetTransfer($it) => $body,
            Directive::BudgetClose($it) => $body,
            Directive::Option(_) | Directive::Plugin(_) | Directive::Include(_) | Directive::Comment(_) => $otherwise,
        }
    };
}

impl Directive {
    pub fn datetime(&self) -> Option<NaiveDateTime> {
        when_dated!(self, |directive| Some(directive.date.naive_datetime()), None)
    }

    /// Mutable access to a directive's date, if it has one.
    pub fn date_mut(&mut self) -> Option<&mut Date> {
        when_dated!(self, |directive| Some(&mut directive.date), None)
    }

    pub fn directive_type(&self) -> DirectiveType {
        self.into()
    }

    pub fn set_meta(mut self, meta: Meta) -> Self {
        if let Some(it) = self.meta_mut() {
            *it = meta;
        }
        self
    }

    /// A directive's metadata, if it carries any: every directive with a date does, and so does a plugin.
    pub fn meta(&self) -> Option<&Meta> {
        match self {
            Directive::Plugin(directive) => Some(&directive.meta),
            _ => when_dated!(self, |directive| Some(&directive.meta), None),
        }
    }

    /// Mutable access to a directive's metadata, if it carries any.
    pub fn meta_mut(&mut self) -> Option<&mut Meta> {
        match self {
            Directive::Plugin(directive) => Some(&mut directive.meta),
            _ => when_dated!(self, |directive| Some(&mut directive.meta), None),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum StringOrAccount {
    String(ZhangString),
    Account(Account),
}

#[derive(Debug, PartialEq, Clone, Eq, Serialize, Deserialize)]
pub enum ZhangString {
    UnquoteString(String),
    QuoteString(String),
}

impl ZhangString {
    pub fn as_str(&self) -> &str {
        match self {
            ZhangString::UnquoteString(s) => s,
            ZhangString::QuoteString(s) => s,
        }
    }
    pub fn to_plain_string(self) -> String {
        match self {
            ZhangString::UnquoteString(unquote) => unquote,
            ZhangString::QuoteString(quote) => quote,
        }
    }
    pub fn quote(content: impl Into<String>) -> ZhangString {
        ZhangString::QuoteString(content.into())
    }
    pub fn unquote(content: impl Into<String>) -> ZhangString {
        ZhangString::UnquoteString(content.into())
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum SingleTotalPrice {
    Single(Amount),
    Total(Amount),
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, Clone)]
pub enum Flag {
    Okay,
    Warning,

    /// `P`: a padding transaction, which a `balance ... with pad` books
    BalancePad,

    /// `C`: the flag zhang once gave a balance check's correcting transaction. A check books nothing
    /// now; a transaction written with this flag (beancount's conversions) is an ordinary transaction
    BalanceCheck,

    Custom(String),
}

impl FromStr for Flag {
    type Err = ErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "*" => Ok(Flag::Okay),
            "!" => Ok(Flag::Warning),
            "P" => Ok(Flag::BalancePad),
            "C" => Ok(Flag::BalanceCheck),
            _ => Ok(Flag::Custom(s.to_owned())),
        }
    }
}
impl Display for Flag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Flag::Okay => "*".to_owned(),
            Flag::Warning => "!".to_owned(),
            Flag::BalancePad => "P".to_owned(),
            Flag::BalanceCheck => "C".to_owned(),
            Flag::Custom(s) => s.to_owned(),
        };
        write!(f, "{}", str)
    }
}

#[derive(EnumString, Debug, PartialEq, Eq, Deserialize, Serialize, Clone, Copy, Display)]
pub enum Rounding {
    #[strum(serialize = "RoundUp")]
    RoundUp,
    #[strum(serialize = "RoundDown")]
    RoundDown,
}

impl Rounding {
    pub fn to_mode(&self) -> bigdecimal::RoundingMode {
        match self {
            Rounding::RoundUp => bigdecimal::RoundingMode::HalfUp,
            Rounding::RoundDown => bigdecimal::RoundingMode::HalfDown,
        }
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use crate::Flag;

    #[test]
    fn should_parse_flag() {
        assert_eq!(Flag::from_str("A").unwrap(), Flag::Custom("A".to_owned()))
    }
}
