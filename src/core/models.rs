use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use strum_macros::EnumString;

use crate::core::account::Account;
use crate::core::amount::Amount;
use crate::core::data::{
    Balance, Close, Comment, Commodity, Custom, Document, Event, Include, Note, Open, Options, Plugin, Price,
    Transaction,
};

#[derive(Debug, PartialEq, Eq)]
pub enum DirectiveType {
    Open,
    Close,
    Commodity,
    Transaction,
    Balance,
    Note,
    Document,
    Price,
    Event,
    Custom,
    Option,
    Plugin,
    Include,
    Comment,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Directive {
    Open(Open),
    Close(Close),
    Commodity(Commodity),
    Transaction(Transaction),
    Balance(Balance),
    Note(Note),
    Document(Document),
    Price(Price),
    Event(Event),
    Custom(Custom),
    Option(Options),
    Plugin(Plugin),
    Include(Include),
    Comment(Comment),
}

impl Directive {
    pub fn datetime(&self) -> Option<NaiveDateTime> {
        match self {
            Directive::Open(open) => Some(open.date.naive_datetime()),
            Directive::Close(close) => Some(close.date.naive_datetime()),
            Directive::Commodity(commodity) => Some(commodity.date.naive_datetime()),
            Directive::Transaction(txn) => Some(txn.date.naive_datetime()),
            Directive::Balance(balance) => Some(match balance {
                Balance::BalanceCheck(check) => check.date.naive_datetime(),
                Balance::BalancePad(pad) => pad.date.naive_datetime(),
            }),
            Directive::Note(note) => Some(note.date.naive_datetime()),
            Directive::Document(document) => Some(document.date.naive_datetime()),
            Directive::Price(price) => Some(price.date.naive_datetime()),
            Directive::Event(event) => Some(event.date.naive_datetime()),
            Directive::Custom(custom) => Some(custom.date.naive_datetime()),
            Directive::Option(_) => None,
            Directive::Plugin(_) => None,
            Directive::Include(_) => None,
            Directive::Comment(_) => None,
        }
    }
    pub fn directive_type(&self) -> DirectiveType {
        match &self {
            Directive::Open(_) => DirectiveType::Open,
            Directive::Close(_) => DirectiveType::Close,
            Directive::Commodity(_) => DirectiveType::Commodity,
            Directive::Transaction(_) => DirectiveType::Transaction,
            Directive::Balance(_) => DirectiveType::Balance,
            Directive::Note(_) => DirectiveType::Note,
            Directive::Document(_) => DirectiveType::Document,
            Directive::Price(_) => DirectiveType::Price,
            Directive::Event(_) => DirectiveType::Event,
            Directive::Custom(_) => DirectiveType::Custom,
            Directive::Option(_) => DirectiveType::Option,
            Directive::Plugin(_) => DirectiveType::Plugin,
            Directive::Include(_) => DirectiveType::Include,
            Directive::Comment(_) => DirectiveType::Comment,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum StringOrAccount {
    String(ZhangString),
    Account(Account),
}

#[derive(Debug, PartialEq, Clone, Eq)]
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
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SingleTotalPrice {
    Single(Amount),
    Total(Amount),
}

#[derive(EnumString, Debug, PartialEq, Eq, strum_macros::ToString, Deserialize, Serialize, Clone)]
pub enum Flag {
    #[strum(serialize = "*", to_string = "*")]
    Okay,
    #[strum(serialize = "!", to_string = "!")]
    Warning,

    #[strum(serialize = "P", to_string = "BalancePad")]
    BalancePad,
}

#[derive(EnumString, Debug, PartialEq, Eq, strum_macros::ToString, Deserialize, Serialize, Clone, Copy)]
pub enum Rounding {
    #[strum(serialize = "RoundUp", to_string = "RoundUp")]
    RoundUp,
    #[strum(serialize = "RoundDown", to_string = "RoundDown")]
    RoundDown,
}
impl Rounding {
    pub fn is_up(&self) -> bool {
        match self {
            Rounding::RoundUp => true,
            Rounding::RoundDown => false,
        }
    }
}
