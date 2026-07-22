//! Inventory, booking method, and trade-amount / lot inference over the AST.
//!
//! This is accounting-domain logic: it interprets parsed postings to infer trade
//! amounts, resolve costs, book lots and total inventories. It lived in the
//! `zhang-ast` syntax crate but belongs here in the domain layer.

use std::collections::HashMap;
use std::ops::{AddAssign, Div, Mul, Neg};
use std::str::FromStr;

use bigdecimal::{BigDecimal, One, Signed, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use strum::Display;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Currency, Posting, PostingCost, SingleTotalPrice, Transaction};

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

/// retrieve the lot meta info from posting
#[derive(Debug)]
pub struct LotMeta {
    pub txn_date: NaiveDate,

    pub cost: Option<PostingCost>,
    pub price: Option<Amount>,
}

impl LotMeta {
    pub fn is_default_lot(&self) -> bool {
        self.cost.is_none() && self.price.is_none()
    }
}

/// Inventory likes a warehouse to record how many commodities are used, and how much are they.
///
/// And for investment tracing purpose, we need to record more details about how much we brought the commodity, and when.
/// That's why we need to use `lots` to record the info.
#[derive(Debug, Clone)]
pub struct Inventory {
    pub currencies: HashMap<Currency, BigDecimal>,
}

impl Inventory {
    pub fn add_amount(&mut self, amount: Amount) {
        let target_commodity_amount = self.currencies.entry(amount.commodity).or_default();
        target_commodity_amount.add_assign(amount.number);
    }

    pub(crate) fn pop(&mut self) -> Option<Amount> {
        self.currencies
            .drain()
            .take(1)
            .next()
            .map(|(currency, currency_inventory)| Amount::new(currency_inventory, currency))
    }

    pub fn get_total(&self, currency: &Currency) -> BigDecimal {
        self.currencies.get(currency).cloned().unwrap_or_else(BigDecimal::zero)
    }

    pub fn is_zero(&self) -> bool {
        self.currencies.values().all(|it| it.is_zero())
    }
    pub fn size(&self) -> usize {
        self.currencies.values().filter(|it| !it.is_zero()).collect_vec().len()
    }
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

    /// if cost is not specified, and it can be indicated from price. e.g.
    /// `Assets:Card 1 CNY @ 10 AAA` then cost `10 AAA` can be indicated from single price`@ 10 AAA`
    pub fn costs(&self) -> Option<Amount> {
        self.posting
            .cost
            .as_ref()
            .and_then(|cost| {
                cost.base.clone().map(|base| {
                    if cost.total {
                        base.div(self.posting.units.as_ref().map(|it| it.number.clone()).unwrap_or_else(BigDecimal::one))
                    } else {
                        base
                    }
                })
            })
            .or_else(|| {
                self.posting.price.as_ref().map(|price| match price {
                    SingleTotalPrice::Single(single_price) => single_price.clone(),
                    SingleTotalPrice::Total(total_price) => Amount::new(
                        (&total_price.number).div(self.posting.units.as_ref().map(|it| &it.number).unwrap_or(&BigDecimal::one())),
                        total_price.commodity.clone(),
                    ),
                })
            })
    }
    /// trade amount means the amount used for other postings to calculate balance
    /// 1. if `unit` is null, return null
    /// 2. if `unit` is present,
    ///    2.1 return `unit * cost`, if cost is present
    ///    2.2 return `unit * single_price`, if single price is present
    ///    2.3 return `total_price * unit.sign()`, if total price is present
    ///    2.4 return `unit`, if both cost and price are not present.
    pub fn trade_amount(&self) -> Option<Amount> {
        self.posting
            .units
            .as_ref()
            .map(|unit| match (self.posting.cost.as_ref(), self.posting.price.as_ref()) {
                (Some(PostingCost { base: Some(cost), total, .. }), _) => {
                    if *total {
                        // total cost contributes the signed total, like a total price
                        if unit.number.is_negative() {
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
                        if unit.number.is_negative() {
                            total_price.neg()
                        } else {
                            total_price.clone()
                        }
                    }
                },
                _ => unit.clone(),
            })
    }

    /// infer the trade amount based on other postings, if it's trade amount is null
    pub fn infer_trade_amount(&self) -> Result<Amount, ErrorKind> {
        self.trade_amount().map(Ok).unwrap_or_else(|| {
            // get other postings' trade amount
            let (trade_amount_postings, non_trade_amount_postings): (Vec<Option<Amount>>, Vec<Option<Amount>>) =
                self.txn.txn_postings().iter().map(|it| it.trade_amount()).partition(|it| it.is_some());
            match non_trade_amount_postings.len() {
                0 => unreachable!("txn should not have zero posting"),
                1 => {
                    let mut inventory = Inventory {
                        currencies: Default::default(),
                    };
                    for trade_amount in trade_amount_postings.into_iter().flatten() {
                        inventory.add_amount(trade_amount);
                    }
                    match inventory.size() {
                        0 => Err(ErrorKind::TransactionCannotInferTradeAmount),
                        1 => {
                            if let Some(inventory_balance) = inventory.pop() {
                                Ok(inventory_balance.neg())
                            } else {
                                Err(ErrorKind::TransactionCannotInferTradeAmount)
                            }
                        }
                        _ => Err(ErrorKind::TransactionExplicitPostingHaveMultipleCommodity),
                    }
                }
                _ => Err(ErrorKind::TransactionHasMultipleImplicitPosting),
            }
        })
    }

    /// return meta of lots, using to generate lot's record
    pub fn lot_meta(&self) -> LotMeta {
        if let Some(unit) = &self.posting.units {
            LotMeta {
                txn_date: self.txn.date.naive_date(),

                cost: self.posting.cost.clone().map(|mut cost| {
                    if cost.total {
                        // normalise total cost to per-unit for lot bookkeeping
                        cost.base = cost.base.map(|base| base.div(unit.number.clone()));
                        cost.total = false;
                    }
                    cost
                }),
                price: self.posting.price.clone().map(|price| match price {
                    SingleTotalPrice::Single(amount) => amount,

                    SingleTotalPrice::Total(amount) => amount.div(unit.number.clone()),
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

/// Trade-amount / inventory inference over a whole [`Transaction`]. These used to
/// be inherent methods on the AST `Transaction`; they are domain logic and live here.
pub trait TransactionInference {
    fn txn_postings(&self) -> Vec<TxnPosting<'_>>;
    fn get_postings_inventory(&self) -> Result<Inventory, ErrorKind>;
}

impl TransactionInference for Transaction {
    fn txn_postings(&self) -> Vec<TxnPosting<'_>> {
        self.postings.iter().map(|posting| TxnPosting { txn: self, posting }).collect_vec()
    }

    fn get_postings_inventory(&self) -> Result<Inventory, ErrorKind> {
        let mut inventory = Inventory {
            currencies: Default::default(),
        };
        for posting in self.txn_postings() {
            let amount = posting.infer_trade_amount()?;
            inventory.add_amount(amount);
        }
        // todo work with commodity precision
        Ok(inventory)
    }
}
