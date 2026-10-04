//! Inventory, booking method, and trade-amount / lot inference over the AST.
//!
//! This is accounting-domain logic: it interprets parsed postings to compute their
//! written trade amounts (weights), resolve costs and lot metas. It lived in the
//! `zhang-ast` syntax crate but belongs here in the domain layer.

use std::ops::{Div, Mul};
use std::str::FromStr;

use bigdecimal::{BigDecimal, Signed, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use strum::Display;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Posting, PostingCost, SingleTotalPrice, Transaction};

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, Clone, Copy, Display)]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum BookingMethod {
    Strict,
    Fifo,
    Lifo,
    Average,
    AverageOnly,
    None,
}

impl FromStr for BookingMethod {
    type Err = ErrorKind;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "STRICT" => Ok(BookingMethod::Strict),
            "FIFO" => Ok(BookingMethod::Fifo),
            "LIFO" => Ok(BookingMethod::Lifo),
            "AVERAGE" => Ok(BookingMethod::Average),
            "AVERAGE_ONLY" => Ok(BookingMethod::AverageOnly),
            "NONE" => Ok(BookingMethod::None),
            _ => Err(ErrorKind::ParseInvalidMeta),
        }
    }
}

impl BookingMethod {
    /// whether booking implements the method. `NONE`, `AVERAGE` and `AVERAGE_ONLY` parse but are
    /// not implemented
    pub fn is_supported(self) -> bool {
        matches!(self, BookingMethod::Strict | BookingMethod::Fifo | BookingMethod::Lifo)
    }

    /// resolve a written booking method (an account's `booking_method` meta or the
    /// `default_booking_method` option). A value that is not a booking method
    /// ([`ErrorKind::ParseInvalidMeta`]) or a method booking does not implement
    /// ([`ErrorKind::UnsupportedBookingMethod`]) resolves to `fallback`, with the error to report
    pub fn resolve(value: &str, fallback: BookingMethod) -> (BookingMethod, Option<ErrorKind>) {
        match BookingMethod::from_str(value) {
            Ok(method) if method.is_supported() => (method, None),
            Ok(_) => (fallback, Some(ErrorKind::UnsupportedBookingMethod)),
            Err(kind) => (fallback, Some(kind)),
        }
    }
}

/// retrieve the lot meta info from posting
#[derive(Debug)]
pub struct LotMeta {
    pub txn_date: NaiveDate,

    pub cost: Option<PostingCost>,
    pub price: Option<Amount>,
}

/// A borrowed view pairing a [`Transaction`] with one of its [`Posting`]s; hosts
/// the trade-amount / cost / lot inference.
#[derive(Debug, PartialEq, Eq)]
pub struct TxnPosting<'a> {
    pub txn: &'a Transaction,
    pub posting: &'a Posting,
}

impl TxnPosting<'_> {
    pub fn units(&self) -> Option<Amount> {
        self.posting.units.clone()
    }

    /// trade amount means the amount used for other postings to calculate balance: the posting's
    /// weight as written
    /// 1. if `unit` is null, return null
    /// 2. if `unit` is present,
    ///    2.1 return `unit * cost`, if a cost with a number is present
    ///    2.2 return `unit * single_price`, if single price is present
    ///    2.3 return `total_price * unit.sign()`, if total price is present (zero for zero units,
    ///    like beancount, whose per-unit price of zero units is zero)
    ///    2.4 return `unit`, if both cost and price are not present.
    ///
    /// A total cost weighs its written total, signed by the units, like a total price: zero for
    /// zero units.
    ///
    /// A cost without a number (`{}`) also returns `unit`; its real weight comes from the lots it
    /// books against, which only booking knows (`Booker::book`).
    pub fn trade_amount(&self) -> Option<Amount> {
        self.posting
            .units
            .as_ref()
            .map(|unit| match (self.posting.cost.as_ref(), self.posting.price.as_ref()) {
                (Some(PostingCost { base: Some(cost), total, .. }), _) => {
                    if *total {
                        // total cost contributes the signed total, like a total price
                        if unit.number.is_zero() {
                            Amount::zero(cost.commodity.clone())
                        } else if unit.number.is_negative() {
                            cost.neg()
                        } else {
                            cost.clone()
                        }
                    } else {
                        Amount::new((&unit.number).mul(&cost.number), cost.commodity.clone())
                    }
                }
                (None, Some(price)) => match price {
                    SingleTotalPrice::Single(single_price) => Amount::new((&unit.number).mul(&single_price.number), single_price.commodity.clone()),
                    SingleTotalPrice::Total(total_price) => {
                        if unit.number.is_zero() {
                            Amount::zero(total_price.commodity.clone())
                        } else if unit.number.is_negative() {
                            total_price.neg()
                        } else {
                            total_price.clone()
                        }
                    }
                },
                _ => unit.clone(),
            })
    }

    /// return meta of lots, using to generate lot's record
    pub fn lot_meta(&self) -> LotMeta {
        if let Some(unit) = &self.posting.units {
            LotMeta {
                txn_date: self.txn.date.naive_date(),

                cost: self.posting.cost.clone().map(|cost| normalise_cost(cost, &unit.number)),
                price: self.posting.price.clone().map(|price| match price {
                    SingleTotalPrice::Single(amount) => amount,

                    SingleTotalPrice::Total(amount) => per_unit_price(&amount, &unit.number),
                }),
            }
        } else {
            LotMeta {
                txn_date: self.txn.date.naive_date(),

                cost: None,
                price: None,
            }
        }
    }

    pub fn account_name(&self) -> String {
        self.posting.account.content.clone()
    }
}

/// `cost` as lots keep it: a total cost (`{{T}}`) becomes the per-unit cost of `units`
pub(crate) fn normalise_cost(mut cost: PostingCost, units: &BigDecimal) -> PostingCost {
    if cost.total {
        // normalise total cost to per-unit for lot bookkeeping
        cost.base = cost.base.map(|base| per_unit_cost(base, units));
        cost.total = false;
    }
    cost
}

/// the per-unit cost of a total cost spread over `units`: `|T| / |units|`, so a sale written
/// `-3 USD {{99 CNY}}` books against the lot bought at `33 CNY`, as in beancount, and a lot never
/// carries a negative cost. Zero units have no per-unit cost: they book nothing, so the written
/// total is kept instead of dividing by zero
fn per_unit_cost(total: Amount, units: &BigDecimal) -> Amount {
    if units.is_zero() {
        total
    } else {
        Amount::new(total.number.abs().div(units.abs()), total.commodity)
    }
}

/// the per-unit price of a total price spread over `units`; zero for zero units, like beancount
fn per_unit_price(total: &Amount, units: &BigDecimal) -> Amount {
    if units.is_zero() {
        Amount::zero(total.commodity.clone())
    } else {
        Amount::new((&total.number).div(units), total.commodity.clone())
    }
}

/// Postings of a whole [`Transaction`]. This used to be an inherent method on the AST `Transaction`;
/// it is domain logic and lives here. Interpolating the implicit posting needs the lots, so it is
/// booking's job (`Booker::book`)
pub trait TransactionInference {
    fn txn_postings(&self) -> Vec<TxnPosting<'_>>;
}

impl TransactionInference for Transaction {
    fn txn_postings(&self) -> Vec<TxnPosting<'_>> {
        self.postings.iter().map(|posting| TxnPosting { txn: self, posting }).collect_vec()
    }
}
