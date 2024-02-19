use itertools::Either;
use zhang_ast::amount::Amount;
use zhang_ast::{Account, Date, Directive, Meta};

pub type BeancountDirective = Either<Directive, BeancountOnlyDirective>;

#[derive(Debug, PartialEq, Eq)]
pub enum BeancountOnlyDirective {
    PushTag(String),
    PopTag(String),
    Pad(PadDirective),
    Balance(BalanceDirective),
}

impl BeancountOnlyDirective {
    pub fn set_meta(mut self, meta: Meta) -> Self {
        match &mut self {
            BeancountOnlyDirective::PushTag(_) => {}
            BeancountOnlyDirective::PopTag(_) => {}
            BeancountOnlyDirective::Pad(directive) => directive.meta = meta,
            BeancountOnlyDirective::Balance(directive) => directive.meta = meta,
        }
        self
    }
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
