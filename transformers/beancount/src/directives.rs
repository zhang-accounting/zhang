use zhang_ast::{Account, Date, Meta};
use zhang_ast::amount::Amount;

#[derive(Debug, PartialEq, Eq)]
pub enum BeancountDirective {
    PushTag(String),
    PopTag(String),
    Pad(PadDirective),
    Balance(BalanceDirective)
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PadDirective {
    pub date: Date,
    pub account: Account,
    pub pad: Account,

    pub meta: Meta,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct BalanceDirective {
    pub date: Date,
    pub account: Account,
    pub amount: Amount,

    pub meta: Meta,
}
