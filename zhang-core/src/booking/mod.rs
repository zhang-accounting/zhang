//! Booking: matching each posting against the lots of its account (booking-split design, #423).
//!
//! [`Booker`] is a pure fold over the directive stream. It sees the `open`s (for the per-account
//! booking methods) and the postings of every transaction, in stream order, and keeps the lots of
//! every account itself. It never reads or writes the [`Store`](crate::store::Store) and needs no
//! timezone: the store fold publishes [`Booker::into_lots`] as `Store.commodity_lots` at its end.
//!
//! This is the booking the store fold used to do inline, moved as is. Its known quirks (E1-E10 in
//! the design, pinned by `tests/booking.rs`) are kept on purpose; fixing them is left to separate,
//! behavior-changing PRs.

use std::collections::HashMap;
use std::ops::{Add, AddAssign, Mul, Neg};
use std::str::FromStr;

use bigdecimal::{BigDecimal, One, Signed, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
use zhang_ast::{Open, PostingCost};

use crate::inventory::{BookingMethod, TxnPosting};
use crate::store::CommodityLotRecord;
use crate::utils::hashmap::HashMapOfExt;

/// the account meta key holding the account's booking method
const BOOKING_METHOD_META: &str = "booking_method";

/// booking state of the store fold: booking methods and lots, per account
pub(crate) struct Booker {
    /// the `default_booking_method` option
    default_method: BookingMethod,
    /// the `booking_method` meta of each account, folded from `open`s in stream order. An invalid
    /// value stays an error until a posting on the account needs it (E7)
    methods: HashMap<String, Result<BookingMethod, ErrorKind>>,
    /// lots per account. Insertion order is the order FIFO/LIFO pick lots in (E10)
    lots: HashMap<String, Vec<CommodityLotRecord>>,
}

/// what booking one posting produced, besides the change to the lots
#[derive(Debug)]
pub(crate) struct PostingBooking {
    /// the posting's share of the transaction's number-sum balance re-check (E2/E3): lot cost ×
    /// units for each lot a cost posting touches, units otherwise
    pub weight: BigDecimal,
    /// booking problems to report against the transaction, in order
    pub errors: Vec<BookingError>,
}

/// a booking problem, reported as a store error of the transaction
#[derive(Debug)]
pub(crate) struct BookingError {
    pub kind: ErrorKind,
    pub metas: HashMap<String, String>,
}

impl Booker {
    pub(crate) fn new(default_method: BookingMethod) -> Self {
        Self {
            default_method,
            methods: HashMap::new(),
            lots: HashMap::new(),
        }
    }

    /// fold an `open`: its `booking_method` meta, if any, becomes the account's booking method.
    /// Like the account meta in the store, a later `open` of the account overrides the value, and
    /// an `open` without the meta keeps it
    pub(crate) fn apply_open(&mut self, open: &Open) {
        if let Some(value) = open.meta.get_all(BOOKING_METHOD_META).last() {
            self.methods.insert(open.account.name().to_owned(), BookingMethod::from_str(value.as_str()));
        }
    }

    /// book one posting of a transaction against the lots of its account. `units` are the
    /// posting's units, or its interpolated amount when it was written without units.
    ///
    /// - `Err(kind)`: the account's `booking_method` meta is invalid; the load aborts (E7)
    /// - panics on a cost posting when the booking method is not FIFO or LIFO (E1)
    pub(crate) fn book_posting(&mut self, txn_posting: &TxnPosting<'_>, units: &Amount) -> Result<PostingBooking, ErrorKind> {
        let account = txn_posting.account_name();
        let lot_meta = txn_posting.lot_meta();
        let booking_method = self.booking_method(&account)?;

        let mut weight = BigDecimal::zero();
        let mut errors = vec![];

        // handle implicit posting cost
        if let Some(cost) = lot_meta.cost {
            let mut accr_amount = units.number.clone();
            loop {
                let target_lot_record = self.lot_by_meta(&account, &units.commodity, &cost, txn_posting.txn.date.naive_date(), booking_method);
                let calculated = (&target_lot_record.amount).add(&accr_amount);
                if !calculated.is_negative() {
                    // the calculated amount is positive, means it is normal case
                    self.update_lot(&account, &target_lot_record, &calculated);

                    weight.add_assign(accr_amount.mul(target_lot_record.cost.map(|it| it.number).unwrap_or(BigDecimal::one())));
                    break;
                } else if target_lot_record.amount.is_zero() {
                    // insert error no enough lot record
                    errors.push(BookingError {
                        kind: ErrorKind::NoEnoughCommodityLot,
                        metas: HashMap::of(
                            // "original_amount",
                            // target_lot_record.amount.to_string(),
                            "transaction_amount",
                            units.number.to_string(),
                        ),
                    });
                    // persist the calculated result even if there is an error
                    self.update_lot(&account, &target_lot_record, &calculated);
                    weight.add_assign(accr_amount.mul(target_lot_record.cost.map(|it| it.number).unwrap_or(BigDecimal::one())));
                    break;
                } else {
                    // if calculated amount is negative, means the matched lots record has no enough amount to do reduction
                    // then set lots record's amount to zero( delete it)
                    self.update_lot(&account, &target_lot_record, &BigDecimal::zero());

                    weight.add_assign(
                        (&target_lot_record.amount)
                            .mul(target_lot_record.cost.map(|it| it.number).unwrap_or(BigDecimal::one()))
                            .neg(),
                    );
                    // subtract the accr amount
                    accr_amount.add_assign(&target_lot_record.amount);
                }
            }
        } else {
            // reduction in default lot
            let target_lot_record = self.default_lot(&account, &units.commodity);

            self.update_lot(&account, &target_lot_record, &(&target_lot_record.amount).add(&units.number));

            weight.add_assign(&units.number);
        }

        Ok(PostingBooking { weight, errors })
    }

    /// the lots of every account the fold booked a posting on, in lot order
    pub(crate) fn into_lots(self) -> HashMap<String, Vec<CommodityLotRecord>> {
        self.lots
    }

    fn booking_method(&self, account_name: &str) -> Result<BookingMethod, ErrorKind> {
        match self.methods.get(account_name) {
            Some(method) => method.clone(),
            None => Ok(self.default_method),
        }
    }

    fn default_lot(&mut self, account_name: &str, currency: &str) -> CommodityLotRecord {
        let entry = self.lots.entry(account_name.to_owned()).or_default();

        let option = entry
            .iter()
            // match commodity
            .filter(|lot| lot.commodity.eq(currency))
            // default lots have none cost
            .filter(|it| it.cost.is_none())
            // default lots have none acquisition date
            .find(|it| it.acquisition_date.is_none())
            .cloned();

        if let Some(record) = option {
            record
        } else {
            // if target lot record does not exist, insert a new one and return it
            let new_lot_record = CommodityLotRecord {
                commodity: currency.to_owned(),
                amount: BigDecimal::zero(),
                acquisition_date: None,
                cost: None,
            };
            entry.push(new_lot_record.clone());
            new_lot_record
        }
    }

    fn lot_by_meta(
        &mut self, account_name: &str, currency: &str, lot_meta: &PostingCost, txn_date: NaiveDate, booking_method: BookingMethod,
    ) -> CommodityLotRecord {
        let entry = self.lots.entry(account_name.to_owned()).or_default();

        let mut option = entry
            .iter()
            // match commodity
            .filter(|lot| lot.commodity.eq(currency))
            // match cost, works with empty cost
            .filter(|it| {
                if lot_meta.base.is_some() {
                    it.cost.eq(&lot_meta.base)
                } else {
                    it.cost.is_some()
                }
            })
            // match cost date
            .filter(|it| {
                if lot_meta.base.is_some() {
                    // if cost date in lot meta is defined, use txn date
                    it.acquisition_date.eq(&lot_meta.date.as_ref().map(|it| it.naive_date()).or(Some(txn_date)))
                } else {
                    // if cost  in meta is null, return all lots
                    true
                }
            });

        let lot_record = match booking_method {
            BookingMethod::Fifo => option.next().cloned(),
            BookingMethod::Lifo => option.next_back().cloned(),
            BookingMethod::Average => {
                unimplemented!()
            }
            BookingMethod::AverageOnly => {
                unimplemented!()
            }
            BookingMethod::Strict => {
                unimplemented!()
            }
            BookingMethod::None => {
                unimplemented!()
            }
        };
        if let Some(record) = lot_record {
            record
        } else {
            // if target lot record does not exist, insert a new one and return it
            let new_lot_record = CommodityLotRecord {
                commodity: currency.to_owned(),
                amount: BigDecimal::zero(),

                // get cost date as acquisition date if persists,
                // if cost is defined, use txn date as acquisition date
                acquisition_date: lot_meta
                    .date
                    .as_ref()
                    .map(|it| it.naive_date())
                    .or_else(|| lot_meta.base.as_ref().map(|_| txn_date)),
                cost: lot_meta.base.clone(),
            };
            entry.push(new_lot_record.clone());
            new_lot_record
        }
    }

    /// set the amount of the first lot equal to `lot_record`; a zero amount removes the lot
    fn update_lot(&mut self, account_name: &str, lot_record: &CommodityLotRecord, amount: &BigDecimal) {
        let entry = self.lots.entry(account_name.to_owned()).or_default();

        if amount.is_zero() {
            // if amount is zero, remove the lot's record
            let pos = entry.iter().find_position(|it| it.eq(&lot_record));
            if let Some((idx, _)) = pos {
                entry.remove(idx);
            }
        } else {
            let option = entry.iter_mut().find(|lot| lot.eq(&lot_record));
            if let Some(lot) = option {
                lot.amount = amount.clone();
            }
        }
    }
}
