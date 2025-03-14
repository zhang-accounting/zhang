use std::fmt::Display;
use std::str::FromStr;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

use crate::account::Account;
use crate::amount::Amount;
use crate::data::{Close, Comment, Commodity, Custom, Document, Event, Include, Note, Open, Options, Plugin, Price, Transaction};
use crate::error::ErrorKind;
use crate::{BalanceCheck, BalancePad, Budget, BudgetAdd, BudgetClose, BudgetTransfer, Meta};

#[derive(Debug, PartialEq, Eq, EnumString, Display)]
pub enum DirectiveType {
    Open,
    Close,
    Commodity,
    Transaction,
    BalancePad,
    BalanceCheck,
    Note,
    Document,
    Price,
    Event,
    Custom,
    Option,
    Plugin,
    Include,
    Comment,

    Budget,
    BudgetAdd,
    BudgetTransfer,
    BudgetClose,
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Directive {
    Open(Open),
    Close(Close),
    Commodity(Commodity),
    Transaction(Transaction),
    BalancePad(BalancePad),
    BalanceCheck(BalanceCheck),
    Note(Note),
    Document(Document),
    Price(Price),
    Event(Event),
    Custom(Custom),
    Option(Options),
    Plugin(Plugin),
    Include(Include),
    Comment(Comment),

    Budget(Budget),
    BudgetAdd(BudgetAdd),
    BudgetTransfer(BudgetTransfer),
    BudgetClose(BudgetClose),
}

impl Directive {
    pub fn datetime(&self) -> Option<NaiveDateTime> {
        match self {
            Directive::Open(open) => Some(open.date.naive_datetime()),
            Directive::Close(close) => Some(close.date.naive_datetime()),
            Directive::Commodity(commodity) => Some(commodity.date.naive_datetime()),
            Directive::Transaction(txn) => Some(txn.date.naive_datetime()),
            Directive::BalanceCheck(check) => Some(check.date.naive_datetime()),
            Directive::BalancePad(pad) => Some(pad.date.naive_datetime()),
            Directive::Note(note) => Some(note.date.naive_datetime()),
            Directive::Document(document) => Some(document.date.naive_datetime()),
            Directive::Price(price) => Some(price.date.naive_datetime()),
            Directive::Event(event) => Some(event.date.naive_datetime()),
            Directive::Custom(custom) => Some(custom.date.naive_datetime()),
            Directive::Option(_) => None,
            Directive::Plugin(_) => None,
            Directive::Include(_) => None,
            Directive::Comment(_) => None,

            Directive::Budget(budget) => Some(budget.date.naive_datetime()),
            Directive::BudgetAdd(budget_add) => Some(budget_add.date.naive_datetime()),
            Directive::BudgetTransfer(budget_transfer) => Some(budget_transfer.date.naive_datetime()),
            Directive::BudgetClose(budget_close) => Some(budget_close.date.naive_datetime()),
        }
    }
    pub fn directive_type(&self) -> DirectiveType {
        match &self {
            Directive::Open(_) => DirectiveType::Open,
            Directive::Close(_) => DirectiveType::Close,
            Directive::Commodity(_) => DirectiveType::Commodity,
            Directive::Transaction(_) => DirectiveType::Transaction,
            Directive::Note(_) => DirectiveType::Note,
            Directive::Document(_) => DirectiveType::Document,
            Directive::Price(_) => DirectiveType::Price,
            Directive::Event(_) => DirectiveType::Event,
            Directive::Custom(_) => DirectiveType::Custom,
            Directive::Option(_) => DirectiveType::Option,
            Directive::Plugin(_) => DirectiveType::Plugin,
            Directive::Include(_) => DirectiveType::Include,
            Directive::Comment(_) => DirectiveType::Comment,
            Directive::BalancePad(_) => DirectiveType::BalancePad,
            Directive::BalanceCheck(_) => DirectiveType::BalanceCheck,
            Directive::Budget(_) => DirectiveType::Budget,
            Directive::BudgetAdd(_) => DirectiveType::BudgetAdd,
            Directive::BudgetTransfer(_) => DirectiveType::BudgetTransfer,
            Directive::BudgetClose(_) => DirectiveType::BudgetClose,
        }
    }

    pub fn set_meta(mut self, meta: Meta) -> Self {
        match &mut self {
            Directive::Open(ref mut directive) => directive.meta = meta,
            Directive::Close(ref mut directive) => directive.meta = meta,
            Directive::Commodity(ref mut directive) => directive.meta = meta,
            Directive::Transaction(ref mut directive) => directive.meta = meta,
            Directive::BalancePad(ref mut directive) => directive.meta = meta,
            Directive::BalanceCheck(ref mut directive) => directive.meta = meta,
            Directive::Note(ref mut directive) => directive.meta = meta,
            Directive::Document(ref mut directive) => directive.meta = meta,
            Directive::Price(ref mut directive) => directive.meta = meta,
            Directive::Event(ref mut directive) => directive.meta = meta,
            Directive::Custom(ref mut directive) => directive.meta = meta,
            Directive::Budget(ref mut directive) => directive.meta = meta,
            Directive::BudgetAdd(ref mut directive) => directive.meta = meta,
            Directive::BudgetTransfer(ref mut directive) => directive.meta = meta,
            Directive::BudgetClose(ref mut directive) => directive.meta = meta,
            Directive::Option(_) => {}
            Directive::Plugin(ref mut directive) => directive.meta = meta,
            Directive::Include(_) => {}
            Directive::Comment(_) => {}
        }
        self
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

    BalancePad,

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
