//! Booking: matching each posting against the lots of its account (booking-split design, #423).
//!
//! [`Booker`] is a pure fold over the directive stream. It sees the `open`s (for the per-account
//! booking methods) and every transaction, in stream order, and keeps the lots of every account
//! itself. It never reads or writes the [`Store`](crate::store::Store) and needs no timezone: the
//! store fold publishes [`Booker::into_lots`] as `Store.commodity_lots` at its end.
//!
//! [`Booker::book`] books a whole transaction (E2-E4):
//! 1. the explicit postings whose weight their lots decide, a cost without a number (`{}`), are
//!    booked first: their weight is `units × lot cost` of every lot they reduce. With an implicit
//!    posting this is a dry run; the lots then change in written order, as they always did;
//! 2. every other posting weighs as written: its units, its price-converted units (`@`, `@@`) or
//!    `units × cost` (a cost with a number: every lot it books against has that cost);
//! 3. the single implicit posting gets the negated sum of those weights, which must be in a single
//!    commodity, like beancount's interpolation. A sum with at most [`EXACT_DECIMALS`] decimals
//!    is used exactly as computed, so written amounts and their products are never rounded. A
//!    longer one carries the dust of a division (a `{{T}}` lot cost is `T / units` at 100 digits):
//!    it is rounded at the transaction's scale of the commodity, the larger of the commodity's
//!    precision and the most decimals written in the transaction in that commodity, both when
//!    telling whether it is zero and for the implicit posting's units. When the weights already
//!    balance in a single commodity, the implicit posting books zero of it, so the journal shows
//!    the posting the user wrote (beancount drops a zero auto-posting instead);
//! 4. the sum of all weights, per commodity, is the residual the store fold checks against each
//!    commodity's precision.
//!
//! Booking rewrites the postings in place, beancount-style (design §3, §4): the implicit posting
//! gets its interpolated units, a cost spec becomes the per-unit cost, acquisition date and label
//! of the lot, and a reduction spanning several lots becomes one posting per lot, adjacent. A
//! posting booking changed carries what was written in [`Posting::written`]: the index of the
//! written posting, which the legs of a split share, its units and its cost spec. The store fold
//! groups the legs back into one row per written posting ([`written_groups`], the rule the
//! exporter follows too: a split a stage broke apart is shown as booked). The part of a `{}`
//! reduction no lot covers keeps `{}`, so it books the same lot again (E9). Booking a booked
//! transaction again changes nothing: every leg matches exactly the lot it was booked against, and
//! STRICT's ambiguity check and the error metas use the written form, so the errors are the same.
//! An unbookable transaction is left untouched.
//!
//! Lot matching follows beancount (E5, [`LotFilter`]): the fields a cost spec gives are criteria
//! and the missing ones wildcards, so a reduction `{10 CNY}` matches the lots held at 10 CNY from any
//! acquisition date, and `{, "a"}` the lots labelled `a` whatever their cost (#498). An
//! augmentation opens or extends the lot of exactly what it writes, label included: lots that
//! differ only by label are distinct. Its other known quirks (E6, E8 and E9 in the design, pinned
//! by `tests/booking.rs`) are kept on purpose; fixing them is left to separate, behavior-changing
//! PRs.
//!
//! Booking methods (E1, E7, E10):
//! - `FIFO` and `LIFO` take the matching lots by acquisition date, oldest or newest first, like
//!   beancount ([`pick`]). The lots themselves stay in creation order.
//! - `STRICT` follows beancount's `booking_method_STRICT`: a reduction must match a single lot, or
//!   reduce every lot it matches in full. Any other reduction matching several lots is reported as
//!   [`ErrorKind::AmbiguousLotMatch`] and still booked, like FIFO among the matching lots, so the
//!   ledger keeps its numbers. Augmentations book like FIFO.
//! - `NONE`, `AVERAGE` and `AVERAGE_ONLY` are not implemented. An account using one, or an invalid
//!   value, gets an error at its `open` ([`Booker::apply_open`]) and books with the ledger's default
//!   method.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::{Add, AddAssign, Mul, Neg};

use bigdecimal::{BigDecimal, RoundingMode, Signed, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
pub(crate) use zhang_ast::{group_units, written_groups};
use zhang_ast::{Currency, Date, Open, Posting, PostingCost, Rounding, SingleTotalPrice, Transaction, WrittenPosting};

use crate::constants::DEFAULT_ROUNDING;
use crate::inventory::{normalise_cost, BookingMethod, TransactionInference, TxnPosting};
use crate::store::CommodityLotRecord;
use crate::utils::hashmap::HashMapOfExt;

/// the most decimals a sum of weights has when it comes from written numbers alone. Units, costs
/// and prices are exact decimals, and so are their products and sums, far below 20 decimals in
/// practice. Only a division adds more: a `{{T}}` lot cost is `T / units`, computed at 100 digits,
/// so a sum with more decimals carries division dust and is rounded at the transaction's scale
const EXACT_DECIMALS: i64 = 20;

/// the account meta key holding the account's booking method
const BOOKING_METHOD_META: &str = "booking_method";

/// booking state of the store fold: booking methods and lots, per account
pub(crate) struct Booker {
    /// the `default_booking_method` option
    default_method: BookingMethod,
    /// the booking method of each account with a `booking_method` meta, folded from `open`s in
    /// stream order. An invalid or unsupported value resolves to `default_method` (E1, E7)
    methods: HashMap<String, BookingMethod>,
    /// lots per account, in creation order, the order `Store.commodity_lots` keeps. FIFO and LIFO
    /// pick lots by acquisition date instead ([`pick`], E10)
    lots: HashMap<String, Vec<CommodityLotRecord>>,
    /// precision and rounding of every defined commodity, folded in stream order like the store's
    /// commodities
    precisions: HashMap<Currency, (i64, RoundingMode)>,
}

/// the units of an implicit posting
struct Interpolated {
    /// the units it books: the negated residual, without its division dust ([`round`])
    units: Amount,
    /// its weight in the transaction's residual: the negated residual itself. It balances its
    /// commodity by construction, so rounding its units is not an imbalance
    weight: Amount,
}

/// what booking a transaction produced. Booking is all or nothing: an unbookable transaction
/// leaves the lots as they were
#[derive(Debug)]
pub(crate) enum BookOutcome {
    /// the transaction is booked against the lots
    Booked(BookedTransaction),
    /// the implicit posting cannot be interpolated
    Unbookable {
        /// one of [`ErrorKind::TransactionHasMultipleImplicitPosting`],
        /// [`ErrorKind::TransactionCannotInferTradeAmount`] and
        /// [`ErrorKind::TransactionExplicitPostingHaveMultipleCommodity`]
        kind: ErrorKind,
        /// the booking problems found while weighing the explicit postings against the lots (only
        /// when a cost posting without a number needed it), in posting order. They usually explain
        /// `kind`: the part of a `{}` reduction no lot covers has no cost and weighs its units
        errors: Vec<BookingError>,
    },
}

/// what booking a transaction produced, besides its booked postings (written into the transaction)
#[derive(Debug)]
pub(crate) struct BookedTransaction {
    /// the sum of the weights of all postings, per weight commodity in commodity order. A
    /// commodity whose weights cancel out is still listed, with zero
    pub residual: BTreeMap<Currency, BigDecimal>,
    /// booking problems to report against the transaction, in posting order
    pub errors: Vec<BookingError>,
}

/// what booking one posting produced, besides the change to the lots
#[derive(Debug)]
struct PostingBooking {
    /// the posting's weight, as amounts: one per lot for a posting whose lots decide it
    weight: Vec<Amount>,
    /// booking problems to report against the transaction, in order
    errors: Vec<BookingError>,
    /// the booked form of the posting: one leg per lot it booked against
    legs: Vec<Leg>,
}

/// one booked posting: `units` booked against one lot, with that lot's cost
#[derive(Debug)]
struct Leg {
    units: Amount,
    cost: Option<PostingCost>,
}

/// a booking problem, reported as a store error of the transaction
#[derive(Debug)]
pub(crate) struct BookingError {
    pub kind: ErrorKind,
    pub metas: HashMap<String, String>,
}

/// the lots of some accounts, saved to undo a dry run; `None` for an account without lots
type LotsSnapshot = Vec<(String, Option<Vec<CommodityLotRecord>>)>;

/// the lots a cost posting books against ([`Booker::lot_filter`]). As in beancount, the fields its
/// cost spec gives are criteria and the missing ones wildcards (E5): a reduction `{10 CNY}` matches
/// the lots held at 10 CNY from any acquisition date, `{10 CNY, 2024-05-16}` only the ones acquired
/// that day, `{, "a"}` the lots labelled `a`, and `{}` every lot held at cost
struct LotFilter<'a> {
    commodity: &'a str,
    /// the lots' cost; `None` matches every lot held at cost
    cost: Option<&'a Amount>,
    /// the lots' acquisition date; `None` matches any
    date: Option<NaiveDate>,
    /// the lots' label
    label: LabelFilter<'a>,
}

/// how a [`LotFilter`] matches the label of a lot (#498)
#[derive(Clone, Copy)]
enum LabelFilter<'a> {
    /// any label, or none: a reduction written without a label
    Any,
    /// exactly this label, `None` being a lot without one: a reduction naming a label, or an
    /// augmentation, which opens or extends the lot of exactly the label it writes
    Exactly(Option<&'a str>),
}

impl Booker {
    pub(crate) fn new(default_method: BookingMethod) -> Self {
        Self {
            default_method,
            methods: HashMap::new(),
            lots: HashMap::new(),
            precisions: HashMap::new(),
        }
    }

    /// fold a commodity definition: its precision and rounding, as the store keeps them
    pub(crate) fn define_commodity(&mut self, currency: &str, precision: i32, rounding: Rounding) {
        self.precisions.insert(currency.to_owned(), (i64::from(precision), rounding.to_mode()));
    }

    /// fold an `open`: its `booking_method` meta, if any, becomes the account's booking method.
    /// Like the account meta in the store, a later `open` of the account overrides the value, and
    /// an `open` without the meta keeps it.
    ///
    /// A value that is not a booking method (`ParseInvalidMeta`, E7) or a method booking does not
    /// implement (`UnsupportedBookingMethod`, E1) makes the account book with the default method;
    /// the error, with metas `account_name` and `booking_method`, is for the `open` to report
    pub(crate) fn apply_open(&mut self, open: &Open) -> Option<BookingError> {
        let value = open.meta.get_all(BOOKING_METHOD_META).last()?.as_str().to_owned();
        let account = open.account.name().to_owned();
        let (method, error) = BookingMethod::resolve(&value, self.default_method);
        self.methods.insert(account.clone(), method);
        error.map(|kind| BookingError {
            kind,
            metas: HashMap::of2("account_name", account, BOOKING_METHOD_META, value),
        })
    }

    /// book a transaction: interpolate its implicit posting from the weights of the other postings,
    /// then book every posting against the lots of its account, in written order, and rewrite the
    /// postings into their booked form (see the module docs). An unbookable transaction is left as
    /// it is
    pub(crate) fn book(&mut self, txn: &mut Transaction) -> BookOutcome {
        let postings = txn.txn_postings();
        let interpolated = match postings.iter().filter(|it| it.posting.units.is_none()).count() {
            0 => None,
            1 => match self.interpolate(&postings) {
                Ok(units) => Some(units),
                Err((kind, errors)) => return BookOutcome::Unbookable { kind, errors },
            },
            _ => {
                return BookOutcome::Unbookable {
                    kind: ErrorKind::TransactionHasMultipleImplicitPosting,
                    errors: vec![],
                }
            }
        };

        let indexes = fresh_indexes(&txn.postings);
        let mut booked = BookedTransaction {
            residual: BTreeMap::new(),
            errors: vec![],
        };
        let mut rewritten: Vec<Posting> = Vec::with_capacity(postings.len());
        let mut previous_index: Option<usize> = None;
        for (position, posting) in postings.iter().enumerate() {
            let (units, implicit_weight) = match (posting.units(), &interpolated) {
                (Some(units), _) => (units, None),
                (None, Some(interpolated)) => (interpolated.units.clone(), Some(interpolated.weight.clone())),
                (None, None) => unreachable!("only the implicit posting has no units, and it is interpolated"),
            };
            // the first leg of a booked posting stands for the posting as written
            let index = posting.posting.written.as_ref().map(|it| it.index);
            let first_of_group = index.is_none() || index != previous_index;
            previous_index = index;
            let booking = self.book_posting(posting, &units, first_of_group);
            add_weight(&mut booked.residual, implicit_weight.map(|it| vec![it]).unwrap_or(booking.weight));
            booked.errors.extend(booking.errors);
            rewritten.extend(booked_postings(posting.posting, booking.legs, indexes[position]));
        }
        if let Some(interpolated) = &interpolated {
            // the dry run in `interpolate` booked the explicit postings exactly as this pass did
            debug_assert!(
                booked.residual.get(&interpolated.weight.commodity).is_some_and(|it| it.is_zero()),
                "interpolation must balance its commodity: {:?}",
                booked.residual
            );
        }
        txn.postings = rewritten;
        BookOutcome::Booked(booked)
    }

    /// the units of the transaction's single implicit posting: the negated sum of the weights of
    /// the explicit postings, which must be unbalanced in exactly one commodity, without its
    /// division dust ([`round`]). A commodity whose sum is zero once its dust is gone is balanced.
    /// If every commodity is, the implicit posting books zero of the weights' commodity, provided
    /// they have a single one.
    ///
    /// The weight of a cost posting without a number is only known once it is booked, so when
    /// there is one, the explicit postings are booked as a dry run, and the lots restored after.
    /// On failure, the booking problems of that dry run come with the error
    fn interpolate(&mut self, postings: &[TxnPosting<'_>]) -> Result<Interpolated, (ErrorKind, Vec<BookingError>)> {
        let explicit = postings.iter().filter(|it| it.posting.units.is_some()).collect_vec();
        let mut residual = BTreeMap::new();
        let mut errors = vec![];
        if explicit.iter().any(|it| weighs_by_lots(it.posting)) {
            let snapshot = self.snapshot(postings);
            let mut previous_index: Option<usize> = None;
            for posting in &explicit {
                let units = posting.units().expect("an explicit posting has units");
                let index = posting.posting.written.as_ref().map(|it| it.index);
                let first_of_group = index.is_none() || index != previous_index;
                previous_index = index;
                let booking = self.book_posting(posting, &units, first_of_group);
                add_weight(&mut residual, booking.weight);
                errors.extend(booking.errors);
            }
            self.restore(snapshot);
        } else {
            for posting in &explicit {
                add_weight(&mut residual, posting.trade_amount());
            }
        }

        let written = written_scales(postings);
        let weight_currencies = residual.len();
        let mut unbalanced = residual
            .iter()
            .filter(|(commodity, number)| !round(number, self.scale(commodity, &written)).is_zero());
        match (unbalanced.next(), unbalanced.next()) {
            (Some((commodity, number)), None) => {
                let weight = Amount::new(number.neg(), commodity.clone());
                let units = Amount::new(round(&weight.number, self.scale(commodity, &written)), commodity.clone());
                Ok(Interpolated { units, weight })
            }
            (Some(_), Some(_)) => Err((ErrorKind::TransactionExplicitPostingHaveMultipleCommodity, errors)),
            // already balanced in a single weight commodity, e.g. a sale at cost with an implicit
            // gain: the implicit posting books zero of that commodity, so the journal keeps the
            // posting the user wrote (beancount drops a zero auto-posting instead)
            (None, _) if weight_currencies == 1 => {
                let (commodity, number) = residual.into_iter().next().expect("one weight commodity");
                Ok(Interpolated {
                    units: Amount::new(BigDecimal::zero(), commodity.clone()),
                    weight: Amount::new(number.neg(), commodity),
                })
            }
            // nothing to weigh, or balanced in several commodities: no commodity to give it
            (None, _) => Err((ErrorKind::TransactionCannotInferTradeAmount, errors)),
        }
    }

    /// the scale a transaction interpolates `commodity` at, with the rounding to use: the larger of
    /// the commodity's precision and the most decimals `written` in the transaction in it. An
    /// undefined commodity rounds at the written decimals with the default rounding, and one
    /// neither defined nor written is not rounded
    fn scale(&self, commodity: &str, written: &HashMap<&str, i64>) -> Option<(i64, RoundingMode)> {
        let written = written.get(commodity).copied();
        match self.precisions.get(commodity) {
            Some(&(precision, mode)) => Some((written.map_or(precision, |it| it.max(precision)), mode)),
            None => written.map(|it| (it, DEFAULT_ROUNDING.to_mode())),
        }
    }

    /// save the lots of the accounts of `postings`
    fn snapshot(&self, postings: &[TxnPosting<'_>]) -> LotsSnapshot {
        postings
            .iter()
            .map(|it| it.posting.account.name())
            .unique()
            .map(|account| (account.to_owned(), self.lots.get(account).cloned()))
            .collect()
    }

    /// put back the lots saved by [`Booker::snapshot`]
    fn restore(&mut self, snapshot: LotsSnapshot) {
        for (account, lots) in snapshot {
            match lots {
                Some(lots) => self.lots.insert(account, lots),
                None => self.lots.remove(&account),
            };
        }
    }

    /// book one posting of a transaction against the lots of its account. `units` are the
    /// posting's units, or its interpolated amount when it was written without units.
    /// `first_of_group` says whether the posting is the first leg of the posting it was written
    /// as (always, for a posting booking never changed): the leg STRICT's check and the error
    /// metas refer to the written form from
    fn book_posting(&mut self, txn_posting: &TxnPosting<'_>, units: &Amount, first_of_group: bool) -> PostingBooking {
        let posting = txn_posting.posting;
        let account = txn_posting.account_name();
        let lot_meta = txn_posting.lot_meta();
        let booking_method = self.booking_method(&account);
        let txn_date = txn_posting.txn.date.naive_date();
        // the error metas name the units as written, those of the posting a leg came from
        let written_units = posting.written.as_ref().and_then(|it| it.units.as_ref()).unwrap_or(units);

        // the weight of every lot the posting books against
        let mut lot_weights = vec![];
        let mut legs = vec![];
        let mut errors = vec![];

        // handle implicit posting cost
        if let Some(cost) = lot_meta.cost {
            if booking_method == BookingMethod::Strict && first_of_group {
                errors.extend(self.ambiguous_reduction_of(posting, &account, units, &cost, txn_date));
            }
            let filter = self.lot_filter(&account, units, &cost, txn_date);
            let mut accr_amount = units.number.clone();
            loop {
                let target_lot_record = self.lot_by_meta(&account, &filter, &cost, txn_date, booking_method);
                let calculated = (&target_lot_record.amount).add(&accr_amount);
                if !calculated.is_negative() {
                    // the calculated amount is positive, means it is normal case
                    self.update_lot(&account, &target_lot_record, &calculated);

                    lot_weights.push(lot_weight(&target_lot_record, accr_amount.clone()));
                    legs.push(leg(&target_lot_record, accr_amount, &cost, &units.commodity));
                    break;
                } else if target_lot_record.amount.is_zero() {
                    // insert error no enough lot record
                    errors.push(BookingError {
                        kind: ErrorKind::NoEnoughCommodityLot,
                        metas: HashMap::of(
                            // "original_amount",
                            // target_lot_record.amount.to_string(),
                            "transaction_amount",
                            written_units.number.to_string(),
                        ),
                    });
                    // persist the calculated result even if there is an error
                    self.update_lot(&account, &target_lot_record, &calculated);
                    lot_weights.push(lot_weight(&target_lot_record, accr_amount.clone()));
                    legs.push(leg(&target_lot_record, accr_amount, &cost, &units.commodity));
                    break;
                } else {
                    // if calculated amount is negative, means the matched lots record has no enough amount to do reduction
                    // then set lots record's amount to zero( delete it)
                    self.update_lot(&account, &target_lot_record, &BigDecimal::zero());

                    let taken = (&target_lot_record.amount).neg();
                    lot_weights.push(lot_weight(&target_lot_record, taken.clone()));
                    legs.push(leg(&target_lot_record, taken, &cost, &units.commodity));
                    // subtract the accr amount
                    accr_amount.add_assign(&target_lot_record.amount);
                }
            }
        } else {
            // reduction in default lot
            let target_lot_record = self.default_lot(&account, &units.commodity);

            self.update_lot(&account, &target_lot_record, &(&target_lot_record.amount).add(&units.number));
            legs.push(Leg {
                units: units.clone(),
                // an implicit posting books the default lot whatever cost spec it wrote (the lot
                // meta drops the spec without units): its leg carries none, so booking it again
                // books the same lot, and the spec stays in the written form
                cost: posting.units.as_ref().and(posting.cost.clone()),
            });
        }

        let weight = if weighs_by_lots(posting) {
            lot_weights
        } else {
            // as written; an implicit posting weighs its interpolated units
            vec![txn_posting.trade_amount().unwrap_or_else(|| units.clone())]
        };
        PostingBooking {
            weight,
            errors,
            legs: merge_legs(legs),
        }
    }

    /// STRICT's check of a reduction ([`Booker::ambiguous_reduction`]) for `posting` as written. A
    /// leg booked against a lot carries that lot's full cost and date, so it alone never matches
    /// several lots: for a leg, the check uses the units and cost spec of the posting it came from,
    /// so booking a booked transaction again reports what booking it the first time did
    fn ambiguous_reduction_of(&self, posting: &Posting, account: &str, units: &Amount, cost: &PostingCost, txn_date: NaiveDate) -> Option<BookingError> {
        let (units, cost) = match &posting.written {
            Some(WrittenPosting {
                units: Some(written_units),
                cost: Some(written_cost),
                ..
            }) => (written_units.clone(), normalise_cost(written_cost.clone(), &written_units.number)),
            _ => (units.clone(), cost.clone()),
        };
        let filter = self.lot_filter(account, &units, &cost, txn_date);
        self.ambiguous_reduction(account, &units, &filter)
    }

    /// whether `account` or one of its sub-accounts holds `currency` at cost: a lot of it with a cost
    pub(crate) fn holds_at_cost(&self, account: &str, currency: &str) -> bool {
        let sub_accounts = format!("{account}:");
        self.lots
            .iter()
            .filter(|(name, _)| name.as_str() == account || name.starts_with(&sub_accounts))
            .flat_map(|(_, lots)| lots)
            .any(|lot| lot.commodity == currency && lot.cost.is_some() && !lot.amount.is_zero())
    }

    /// the lots of every account the fold booked a posting on, in lot order
    pub(crate) fn into_lots(self) -> HashMap<String, Vec<CommodityLotRecord>> {
        self.lots
    }

    fn booking_method(&self, account_name: &str) -> BookingMethod {
        self.methods.get(account_name).copied().unwrap_or(self.default_method)
    }

    /// the lots `units` with the cost spec `cost` book against in the account. The posting reduces
    /// when a lot its spec matches holds the opposite sign (like beancount's `is_reduced_by`): a
    /// cost with a number but no date then matches lots of any date, and a spec without a label
    /// lots of any label. Otherwise the posting augments: it adds to the lot of exactly what it
    /// writes, its cost acquired on the transaction's date when it gives no date, and its label or
    /// none, or opens that lot
    fn lot_filter<'a>(&self, account_name: &str, units: &'a Amount, cost: &'a PostingCost, txn_date: NaiveDate) -> LotFilter<'a> {
        let mut filter = LotFilter {
            commodity: &units.commodity,
            cost: cost.base.as_ref(),
            date: cost.date.as_ref().map(|it| it.naive_date()),
            label: match &cost.label {
                Some(label) => LabelFilter::Exactly(Some(label)),
                None => LabelFilter::Any,
            },
        };
        let lots = self.lots.get(account_name).map(Vec::as_slice).unwrap_or_default();
        if !matching_lots(lots, &filter).any(|lot| reduces(lot, &units.number)) {
            if filter.cost.is_some() && filter.date.is_none() {
                filter.date = Some(txn_date);
            }
            filter.label = LabelFilter::Exactly(cost.label.as_deref());
        }
        filter
    }

    /// STRICT, as beancount's `booking_method_STRICT`: a reduction matching several lots must
    /// reduce all of them in full. Otherwise the match is ambiguous and this is the error to
    /// report, with metas `account_name`, `transaction_amount` (the units) and `matched_lots`.
    /// The lots a posting reduces are the matching lots holding the opposite sign: an augmentation
    /// matches none of them
    fn ambiguous_reduction(&self, account_name: &str, units: &Amount, filter: &LotFilter<'_>) -> Option<BookingError> {
        let lots = self.lots.get(account_name)?;
        let reduced = matching_lots(lots, filter).filter(|lot| reduces(lot, &units.number)).collect_vec();
        if reduced.len() < 2 {
            return None;
        }
        let total: BigDecimal = reduced.iter().map(|lot| &lot.amount).sum();
        if (total + &units.number).is_zero() {
            // beancount's exception: a reduction of every matching lot in full is not ambiguous
            return None;
        }
        Some(BookingError {
            kind: ErrorKind::AmbiguousLotMatch,
            metas: HashMap::of3(
                "account_name",
                account_name,
                "transaction_amount",
                units.number.to_string(),
                "matched_lots",
                reduced.iter().map(|lot| describe_lot(lot)).join(", "),
            ),
        })
    }

    fn default_lot(&mut self, account_name: &str, currency: &str) -> CommodityLotRecord {
        let entry = self.lots.entry(account_name.to_owned()).or_default();

        let option = entry
            .iter()
            // match commodity
            .filter(|lot| lot.commodity.eq(currency))
            // default lots have none cost
            .filter(|it| it.cost.is_none())
            // nor a label: `{, "a"}` without a cost opens a labelled lot of its own (#498)
            .filter(|it| it.label.is_none())
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
                label: None,
            };
            entry.push(new_lot_record.clone());
            new_lot_record
        }
    }

    /// the lot matching `filter` that `booking_method` books against first ([`pick`]), or a new
    /// empty lot for `lot_meta`, added after the account's lots
    fn lot_by_meta(
        &mut self, account_name: &str, filter: &LotFilter<'_>, lot_meta: &PostingCost, txn_date: NaiveDate, booking_method: BookingMethod,
    ) -> CommodityLotRecord {
        let entry = self.lots.entry(account_name.to_owned()).or_default();

        let lot_record = pick(matching_lots(entry, filter), booking_method).cloned();
        if let Some(record) = lot_record {
            record
        } else {
            // if target lot record does not exist, insert a new one and return it
            let new_lot_record = CommodityLotRecord {
                commodity: filter.commodity.to_owned(),
                amount: BigDecimal::zero(),

                // get cost date as acquisition date if persists,
                // if cost is defined, use txn date as acquisition date
                acquisition_date: lot_meta
                    .date
                    .as_ref()
                    .map(|it| it.naive_date())
                    .or_else(|| lot_meta.base.as_ref().map(|_| txn_date)),
                cost: lot_meta.base.clone(),
                label: lot_meta.label.clone(),
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

/// the lots matching `filter`, in lot (creation) order. Lots held without cost never match
fn matching_lots<'a>(lots: &'a [CommodityLotRecord], filter: &'a LotFilter<'a>) -> impl Iterator<Item = &'a CommodityLotRecord> + 'a {
    lots.iter().filter(move |lot| {
        lot.commodity == filter.commodity
            && match filter.cost {
                Some(cost) => lot.cost.as_ref() == Some(cost),
                None => lot.cost.is_some(),
            }
            && filter.date.is_none_or(|date| lot.acquisition_date == Some(date))
            && match filter.label {
                LabelFilter::Any => true,
                LabelFilter::Exactly(label) => lot.label.as_deref() == label,
            }
    })
}

/// the lot `booking_method` books against first among `lots`, like beancount's FIFO and LIFO (E10):
/// the one with the oldest acquisition date, or the newest for LIFO. Lots of the same date go in
/// creation order, reversed for LIFO, so LIFO takes the one created last: the order is exactly
/// FIFO's, reversed. STRICT books like FIFO once `ambiguous_reduction` has checked the match. NONE,
/// AVERAGE and AVERAGE_ONLY never get here: they resolve to the default method at the `open`
fn pick<'a>(lots: impl Iterator<Item = &'a CommodityLotRecord>, booking_method: BookingMethod) -> Option<&'a CommodityLotRecord> {
    match booking_method {
        // the last of the equally newest
        BookingMethod::Lifo => lots.max_by_key(|lot| lot.acquisition_date),
        // the first of the equally oldest
        BookingMethod::Fifo | BookingMethod::Strict | BookingMethod::Average | BookingMethod::AverageOnly | BookingMethod::None => {
            lots.min_by_key(|lot| lot.acquisition_date)
        }
    }
}

/// whether booking `units` against `lot` reduces it: they have opposite signs
fn reduces(lot: &CommodityLotRecord, units: &BigDecimal) -> bool {
    (lot.amount.is_positive() && units.is_negative()) || (lot.amount.is_negative() && units.is_positive())
}

/// whether the posting's weight is decided by the lots it books against: an explicit posting with
/// a cost but no cost number (`{}`, `{{}}`, `{date}`). Any other posting weighs as written
fn weighs_by_lots(posting: &Posting) -> bool {
    posting.units.is_some() && posting.cost.as_ref().is_some_and(|cost| cost.base.is_none())
}

/// the weight of `units` booked against `lot`: `units × lot cost` in the cost's commodity, or the
/// units themselves for a lot held without cost
fn lot_weight(lot: &CommodityLotRecord, units: BigDecimal) -> Amount {
    match &lot.cost {
        Some(cost) => Amount::new(units.mul(&cost.number), cost.commodity.clone()),
        None => Amount::new(units, lot.commodity.clone()),
    }
}

/// the leg of a posting booked against `lot`: `units` of `commodity` at the lot's cost, acquisition
/// date and label. A lot without a cost, the one a `{}` reduction no lot covers opens (E9), gives
/// the leg the cost spec as `written`, so booking the leg again opens that lot again
fn leg(lot: &CommodityLotRecord, units: BigDecimal, written: &PostingCost, commodity: &str) -> Leg {
    let cost = match &lot.cost {
        Some(cost) => PostingCost {
            base: Some(cost.clone()),
            date: lot.acquisition_date.map(Date::Date),
            label: lot.label.clone(),
            total: false,
        },
        None => written.clone(),
    };
    Leg {
        units: Amount::new(units, commodity),
        cost: Some(cost),
    }
}

/// `legs` with those booked against the same lot (the same cost) summed into one, in first order
fn merge_legs(legs: Vec<Leg>) -> Vec<Leg> {
    let mut merged: Vec<Leg> = Vec::with_capacity(legs.len());
    for leg in legs {
        match merged.iter_mut().find(|it| it.cost == leg.cost) {
            Some(same) => same.units.number.add_assign(leg.units.number),
            None => merged.push(leg),
        }
    }
    merged
}

/// the booked form of `posting`: one posting per leg, each carrying the written form when booking
/// changed anything (a posting already carrying one keeps it); the posting itself when its single
/// leg is what was written
fn booked_postings(posting: &Posting, legs: Vec<Leg>, index: usize) -> Vec<Posting> {
    if let [only] = legs.as_slice() {
        if Some(&only.units) == posting.units.as_ref() && only.cost == posting.cost {
            return vec![posting.clone()];
        }
    }
    let written = posting.written.clone().unwrap_or_else(|| WrittenPosting {
        index,
        units: posting.units.clone(),
        cost: posting.cost.clone(),
    });
    legs.into_iter()
        .map(|leg| Posting {
            units: Some(leg.units),
            cost: leg.cost,
            written: Some(written.clone()),
            ..posting.clone()
        })
        .collect()
}

/// the `written.index` the posting at each position gets if booking changes it: its position,
/// unless a posting already carries that index (a stage removed postings before a booked one),
/// then an index no posting has, so the legs of different postings never group together
fn fresh_indexes(postings: &[Posting]) -> Vec<usize> {
    let used: HashSet<usize> = postings.iter().filter_map(|it| it.written.as_ref().map(|written| written.index)).collect();
    let mut next = used.iter().max().map_or(0, |max| max + 1).max(postings.len());
    (0..postings.len())
        .map(|position| {
            if used.contains(&position) {
                next += 1;
                next - 1
            } else {
                position
            }
        })
        .collect()
}

/// the most decimals written in the transaction per commodity: of the units, and of the cost and
/// price numbers, per-unit or total, of every posting. `@ 7.12345 CNY` counts 5 for CNY and
/// `{{1000 USD}}` 0 for USD. A booked leg counts what its posting was written as: the lot cost it
/// carries may be a long division (`{{100 CNY}}` over 3 units), which is not a written scale
fn written_scales<'a>(postings: &[TxnPosting<'a>]) -> HashMap<&'a str, i64> {
    let mut scales: HashMap<&str, i64> = HashMap::new();
    for posting in postings {
        let posting = posting.posting;
        let (units, cost) = match &posting.written {
            Some(written) => (written.units.as_ref(), written.cost.as_ref()),
            None => (posting.units.as_ref(), posting.cost.as_ref()),
        };
        let cost = cost.and_then(|cost| cost.base.as_ref());
        let price = posting.price.as_ref().map(|price| match price {
            SingleTotalPrice::Single(amount) | SingleTotalPrice::Total(amount) => amount,
        });
        for amount in units.into_iter().chain(cost).chain(price) {
            let scale = scales.entry(amount.commodity.as_str()).or_insert(0);
            *scale = (*scale).max(amount.number.fractional_digit_count());
        }
    }
    scales
}

/// `number` without its division dust. With at most [`EXACT_DECIMALS`] decimals it is exact and
/// kept as is. With more, it is rounded at `scale` with its rounding, like the balance check rounds
/// a residual, or kept as is without a scale (an undefined commodity not written in the
/// transaction). A trimmed number drops its trailing zeros (`1000.00` is `1000`)
fn round(number: &BigDecimal, scale: Option<(i64, RoundingMode)>) -> BigDecimal {
    if number.fractional_digit_count() <= EXACT_DECIMALS {
        return number.clone();
    }
    let normalized = without_trailing_zeros(number);
    if normalized.fractional_digit_count() <= EXACT_DECIMALS {
        // an exact number written with more digits than it needs
        return normalized;
    }
    match scale {
        Some((scale, mode)) => without_trailing_zeros(&normalized.with_scale_round(scale, mode)),
        None => number.clone(),
    }
}

/// `number` without trailing fractional zeros, and never in exponent form (`1E+3` is `1000`)
fn without_trailing_zeros(number: &BigDecimal) -> BigDecimal {
    let normalized = number.normalized();
    if normalized.fractional_digit_count() < 0 {
        normalized.with_scale(0)
    } else {
        normalized
    }
}

/// add weights to a per-commodity sum
fn add_weight(sum: &mut BTreeMap<Currency, BigDecimal>, weight: impl IntoIterator<Item = Amount>) {
    for amount in weight {
        sum.entry(amount.commodity).or_insert_with(BigDecimal::zero).add_assign(amount.number);
    }
}

/// a lot as `units {cost, acquisition date, "label"}`, for error metas; a lot without a label
/// reads as before
fn describe_lot(lot: &CommodityLotRecord) -> String {
    let label = lot.label.as_ref().map(|label| format!(", \"{label}\"")).unwrap_or_default();
    match (&lot.cost, &lot.acquisition_date) {
        (Some(cost), Some(date)) => format!("{} {} {{{cost}, {date}{label}}}", lot.amount, lot.commodity),
        (Some(cost), None) => format!("{} {} {{{cost}{label}}}", lot.amount, lot.commodity),
        (None, _) => format!("{} {}", lot.amount, lot.commodity),
    }
}

#[cfg(test)]
mod test {
    use bigdecimal::BigDecimal;
    use zhang_ast::amount::Amount;

    use super::Booker;
    use crate::inventory::BookingMethod;
    use crate::store::CommodityLotRecord;

    fn lot(units: i32) -> CommodityLotRecord {
        CommodityLotRecord {
            commodity: "AAPL".to_owned(),
            amount: BigDecimal::from(units),
            cost: Some(Amount::new(BigDecimal::from(100), "USD")),
            acquisition_date: None,
            label: None,
        }
    }

    #[test]
    fn an_empty_lot_is_not_held_at_cost() {
        // the booker drops a lot sold to zero; one left empty still holds nothing
        let mut booker = Booker::new(BookingMethod::Fifo);
        booker.lots.insert("Assets:Stock".to_owned(), vec![lot(0)]);
        assert!(!booker.holds_at_cost("Assets:Stock", "AAPL"));
        booker.lots.insert("Assets:Stock:Sub".to_owned(), vec![lot(2)]);
        assert!(booker.holds_at_cost("Assets:Stock", "AAPL"));
        assert!(!booker.holds_at_cost("Assets:Stock", "USD"));
        assert!(!booker.holds_at_cost("Assets:Stocks", "AAPL"));
    }
}

#[cfg(test)]
mod tests;
