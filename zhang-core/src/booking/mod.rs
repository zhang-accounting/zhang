//! Booking: matching each posting against the lots of its account (booking-split design, #423).
//!
//! [`Booker`] is a pure fold over the directive stream. It sees the `open`s (for the per-account
//! booking methods) and every transaction, in stream order, and keeps the lots of every account
//! itself. It never reads or writes the [`Store`](crate::store::Store) and needs no timezone. It
//! runs twice per load (design §2): pass 1 is the [`BookingStage`](crate::pipeline::BookingStage),
//! before the plugins, whose errors and lots are dropped; pass 2 is the
//! [`ValidateStage`](crate::pipeline::ValidateStage) over the final stream, which leaves booked postings as they are ([`is_booked`]), completes the ones a stage left
//! unbooked, reports every booking error once and publishes [`Booker::into_lots`] as
//! `Store.commodity_lots` at its end.
//!
//! [`Booker::book`] books a whole transaction (E2-E4):
//! 1. the explicit postings whose weight their lots decide, a cost without a number (`{}`), are
//!    booked first: their weight is `units × lot cost` of every lot they reduce. With an implicit
//!    posting this is a dry run; the lots then change in written order, as they always did;
//! 2. every other posting weighs as written: its units, its price-converted units (`@`, `@@`) or
//!    `units × cost` (a cost with a number: every lot it books against has that cost);
//! 3. the single implicit posting gets the negated sum of those weights, which must be in a single
//!    commodity, like beancount's interpolation. A sum of exact weights with at most
//!    [`EXACT_DECIMALS`] decimals is used exactly as computed, so written amounts and their
//!    products are never rounded. A longer one, or one with a weight at a divided cost (a `{{T}}`
//!    lot cost is `T / units` in the 28-digit decimal context of `zhang_shared::decimal`),
//!    carries the dust of a division: it is rounded at the transaction's scale of the commodity,
//!    the larger of the commodity's precision and the most decimals written in the transaction in
//!    that commodity, both when telling whether it is zero and for the implicit posting's units.
//!    When the weights already balance in a single commodity, the implicit posting books zero of
//!    it, so the journal shows the posting the user wrote (beancount drops a zero auto-posting
//!    instead);
//! 4. the sum of all weights, per commodity, is the residual the final validation stage checks against each
//!    commodity's precision.
//!
//! Booking rewrites the postings in place, beancount-style (design §3, §4): the implicit posting
//! gets its interpolated units, a cost spec becomes the per-unit cost, acquisition date and label
//! of the lot, and a reduction spanning several lots becomes one posting per lot, adjacent. A
//! posting booking changed carries what was written in [`Posting::written`]: the index of the
//! written posting, which the legs of a split share, its units and its cost spec. The store fold
//! groups the legs back into one row per written posting ([`written_groups`], the rule the
//! exporter follows too: a split a stage broke apart is shown as booked). A `{}` reduction no lot
//! covers rejects the entire transaction without changing its lots (E9). Booking a booked
//! transaction again changes nothing: every leg matches exactly the lot it was booked against, and
//! STRICT's ambiguity check and the error metas use the written form, so the errors are the same.
//! An unbookable transaction is left untouched.
//!
//! Lot matching follows beancount (E5, [`LotFilter`]): the fields a cost spec gives are criteria
//! and the missing ones wildcards, so a reduction `{10 CNY}` matches the lots held at 10 CNY from any
//! acquisition date, and `{, "a"}` the lots labelled `a` whatever their cost (#498). An
//! augmentation opens or extends the lot of exactly what it writes, label included: lots that
//! differ only by label are distinct. An augmentation with no cost number infers its cost from
//! the other postings' weights (E6), provided the transaction has exactly one missing number.
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
use std::ops::{Add, AddAssign, Neg};

use bigdecimal::{BigDecimal, RoundingMode, Signed, Zero};
use chrono::NaiveDate;
use itertools::Itertools;
use zhang_ast::amount::Amount;
use zhang_ast::error::ErrorKind;
pub(crate) use zhang_ast::{group_units, written_groups};
use zhang_ast::{Currency, Date, Open, Posting, PostingCost, Rounding, SingleTotalPrice, Transaction, WrittenPosting};
use zhang_shared::decimal::{div, mul_in_context, DIVISION_PRECISION};

use crate::constants::DEFAULT_ROUNDING;
use crate::inventory::{normalise_cost, BookingMethod, TransactionInference, TxnPosting};
use crate::store::CommodityLotRecord;
use crate::utils::hashmap::HashMapOfExt;

/// the most decimals a sum of weights has when it comes from written numbers alone. Units, costs
/// and prices are exact decimals, and so are their products and sums, far below 20 decimals in
/// practice. Only a division adds more: a `{{T}}` lot cost is `T / units`, rounded to 28
/// significant digits, so a sum with more decimals carries division dust and is rounded at the
/// transaction's scale. So is a sum with a weight at such a cost ([`Weights::rounded`]): a large
/// cost leaves its dust within these decimals
const EXACT_DECIMALS: i64 = 20;

/// the account meta key holding the account's booking method
const BOOKING_METHOD_META: &str = "booking_method";

/// Booking state of a pipeline stage: booking methods and lots, per account
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
    /// the implicit posting or a cost cannot be resolved
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

type BookingFailure = (ErrorKind, Vec<BookingError>);

/// the lots a cost posting books against ([`Booker::lot_filter`]). As in beancount, the fields its
/// cost spec gives are criteria and the missing ones wildcards (E5): a reduction `{10 CNY}` matches
/// the lots held at 10 CNY from any acquisition date, `{10 CNY, 2024-05-16}` only the ones acquired
/// that day, `{, "a"}` the lots labelled `a`, and `{}` every lot held at cost
struct LotFilter<'a> {
    commodity: &'a str,
    /// the posting's signed units: opposite-sign lots are reduced before same-sign lots
    units: &'a BigDecimal,
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

    /// Validate the residual with the commodity definitions available at this point in the stream.
    /// Undefined commodities take precedence over imbalance, even when their residual is zero.
    pub(crate) fn check_transaction_balance(&self, residual: &BTreeMap<Currency, BigDecimal>) -> Option<ErrorKind> {
        if residual.keys().any(|currency| !self.precisions.contains_key(currency)) {
            return Some(ErrorKind::CommodityDoesNotDefine);
        }
        for (currency, amount) in residual {
            let (precision, rounding) = self.precisions[currency];
            if !amount.with_scale_round(precision, rounding).is_zero() {
                return Some(ErrorKind::UnbalancedTransaction);
            }
        }
        None
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
    /// it is. A posting whose cost spec carries the merge-cost marker (`{*}`) is reported
    /// ([`ErrorKind::CostMergingNotSupported`]), once per posting as written, and books as if the
    /// marker were not there, as in beancount
    pub(crate) fn book(&mut self, txn: &mut Transaction) -> BookOutcome {
        let merge_errors = merge_cost_errors(&txn.postings);
        let mut outcome = self.book_lots(txn);
        if !merge_errors.is_empty() {
            let errors = match &mut outcome {
                BookOutcome::Booked(booked) => &mut booked.errors,
                BookOutcome::Unbookable { errors, .. } => errors,
            };
            errors.splice(0..0, merge_errors);
        }
        outcome
    }

    /// [`Booker::book`] without the merge-cost report
    fn book_lots(&mut self, txn: &mut Transaction) -> BookOutcome {
        let prepared = match self.infer_missing_cost(txn) {
            Ok(prepared) => prepared,
            Err((kind, errors)) => return BookOutcome::Unbookable { kind, errors },
        };
        let input = prepared.as_ref().unwrap_or(txn);
        let postings = input.txn_postings();
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

        // An unresolved cost reduction can fail after earlier postings changed lots. Commit
        // none of the transaction in that case, including a partially consumed lot (E9).
        let snapshot = postings.iter().any(|it| weighs_by_lots(it.posting)).then(|| self.snapshot(&postings));
        let indexes = fresh_indexes(&input.postings);
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
            let booking = match self.book_posting(posting, &units, first_of_group) {
                Ok(booking) => booking,
                Err(errors) => {
                    self.restore(snapshot.expect("only a posting with an unresolved cost can fail"));
                    booked.errors.extend(errors);
                    return BookOutcome::Unbookable {
                        kind: ErrorKind::TransactionCannotInferTradeAmount,
                        errors: booked.errors,
                    };
                }
            };
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

    /// Infer the one missing cost of an augmentation from the other postings' weights (E6).
    /// A reduction must match existing cost lots instead (E9). The common case, one reduction
    /// on each account, needs no extra booking pass; repeated postings on an account are checked
    /// in written order so an earlier purchase can supply the lot a later sale reduces.
    fn infer_missing_cost(&mut self, txn: &Transaction) -> Result<Option<Transaction>, BookingFailure> {
        if !txn.postings.iter().any(weighs_by_lots) {
            return Ok(None);
        }
        let postings = txn.txn_postings();
        let mut seen = HashSet::new();
        let needs_inference = postings.iter().any(|posting| {
            let account = posting.posting.account.name();
            let touched = !seen.insert(account);
            weighs_by_lots(posting.posting) && (touched || !self.is_reduction(account, posting.posting.units.as_ref().expect("explicit units")))
        });
        if !needs_inference {
            return Ok(None);
        }
        let snapshot = self.snapshot(&postings);
        let result = (|| {
            let mut missing = None;
            let mut residual = Weights::default();
            for (index, posting) in postings.iter().enumerate() {
                let Some(units) = posting.units() else { continue };
                if weighs_by_lots(posting.posting) && !self.is_reduction(posting.posting.account.name(), &units) {
                    if missing.is_some() || units.number.is_zero() {
                        return Err((ErrorKind::TransactionCannotInferTradeAmount, vec![]));
                    }
                    missing = Some(index);
                    continue;
                }
                let booking = self
                    .book_posting(posting, &units, true)
                    .map_err(|errors| (ErrorKind::TransactionCannotInferTradeAmount, errors))?;
                residual.add(booking.weight);
            }
            let Some(index) = missing else { return Ok(None) };
            if postings.iter().any(|posting| posting.posting.units.is_none()) {
                return Err((ErrorKind::TransactionCannotInferTradeAmount, vec![]));
            }
            let written = written_scales(&postings);
            let mut unbalanced = residual
                .sums
                .iter()
                .filter(|(commodity, number)| !residual.round(commodity, number, self.scale(commodity, &written)).is_zero());
            let (commodity, weight) = match (unbalanced.next(), unbalanced.next()) {
                (Some(weight), None) => weight,
                (None, _) if residual.sums.len() == 1 => residual.sums.iter().next().expect("one weight commodity"),
                (Some(_), Some(_)) => return Err((ErrorKind::TransactionExplicitPostingHaveMultipleCommodity, vec![])),
                _ => return Err((ErrorKind::TransactionCannotInferTradeAmount, vec![])),
            };
            let units = postings[index].posting.units.as_ref().expect("explicit units");
            // the cost per unit, divided like any other in the 28-digit decimal context
            let rate = div(&weight.neg(), &units.number).expect("the units of a missing cost are not zero");
            if rate.is_negative() {
                return Err((ErrorKind::TransactionCannotInferTradeAmount, vec![]));
            }
            let mut prepared = txn.clone();
            let indexes = fresh_indexes(&txn.postings);
            let posting = &mut prepared.postings[index];
            posting.written.get_or_insert_with(|| WrittenPosting {
                index: indexes[index],
                units: posting.units.clone(),
                cost: posting.cost.clone(),
            });
            let cost = posting.cost.as_mut().expect("a missing cost");
            cost.base = Some(Amount::new(rate, commodity.clone()));
            cost.total = false;
            Ok(Some(prepared))
        })();
        self.restore(snapshot);
        result
    }

    /// Whether units of this commodity reduce any holding, including a holding without cost.
    /// An empty cost on a reduction cannot turn uncosted units into a new cost lot (E9).
    fn is_reduction(&self, account: &str, units: &Amount) -> bool {
        self.lots
            .get(account)
            .into_iter()
            .flatten()
            .any(|lot| lot.commodity == units.commodity && reduces(lot, &units.number))
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
    fn interpolate(&mut self, postings: &[TxnPosting<'_>]) -> Result<Interpolated, BookingFailure> {
        let explicit = postings.iter().filter(|it| it.posting.units.is_some()).collect_vec();
        let mut residual = Weights::default();
        let mut errors = vec![];
        if explicit.iter().any(|it| weighs_by_lots(it.posting)) {
            let snapshot = self.snapshot(postings);
            let mut failed = false;
            let mut previous_index: Option<usize> = None;
            for posting in &explicit {
                let units = posting.units().expect("an explicit posting has units");
                let index = posting.posting.written.as_ref().map(|it| it.index);
                let first_of_group = index.is_none() || index != previous_index;
                previous_index = index;
                let booking = match self.book_posting(posting, &units, first_of_group) {
                    Ok(booking) => booking,
                    Err(failure) => {
                        errors.extend(failure);
                        failed = true;
                        break;
                    }
                };
                residual.add(booking.weight);
                errors.extend(booking.errors);
            }
            self.restore(snapshot);
            if failed {
                return Err((ErrorKind::TransactionCannotInferTradeAmount, errors));
            }
        } else {
            for posting in &explicit {
                residual.add(posting.trade_amount());
            }
        }

        let written = written_scales(postings);
        let weight_currencies = residual.sums.len();
        let mut unbalanced = residual
            .sums
            .iter()
            .filter(|(commodity, number)| !residual.round(commodity, number, self.scale(commodity, &written)).is_zero());
        match (unbalanced.next(), unbalanced.next()) {
            (Some((commodity, number)), None) => {
                let weight = Amount::new(number.neg(), commodity.clone());
                let units = Amount::new(residual.round(commodity, &weight.number, self.scale(commodity, &written)), commodity.clone());
                Ok(Interpolated { units, weight })
            }
            (Some(_), Some(_)) => Err((ErrorKind::TransactionExplicitPostingHaveMultipleCommodity, errors)),
            // already balanced in a single weight commodity, e.g. a sale at cost with an implicit
            // gain: the implicit posting books zero of that commodity, so the journal keeps the
            // posting the user wrote (beancount drops a zero auto-posting instead)
            (None, _) if weight_currencies == 1 => {
                let (commodity, number) = residual.sums.into_iter().next().expect("one weight commodity");
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
    fn book_posting(&mut self, txn_posting: &TxnPosting<'_>, units: &Amount, first_of_group: bool) -> Result<PostingBooking, Vec<BookingError>> {
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
                if reduces(&target_lot_record, &accr_amount) && accr_amount.abs() > target_lot_record.amount.abs() {
                    // The reduction exceeds this lot's absolute units. Consume it completely
                    // and book the signed remainder against the next lot, long or short.
                    self.update_lot(&account, &target_lot_record, &BigDecimal::zero());
                    let taken = (&target_lot_record.amount).neg();
                    lot_weights.push(lot_weight(&target_lot_record, taken.clone()));
                    legs.push(leg(&target_lot_record, taken, &cost, &units.commodity));
                    accr_amount.add_assign(&target_lot_record.amount);
                    continue;
                }
                if target_lot_record.amount.is_zero() && accr_amount.is_negative() {
                    // Keep the existing warning when a sale opens a short lot.
                    errors.push(BookingError {
                        kind: ErrorKind::NoEnoughCommodityLot,
                        metas: HashMap::of("transaction_amount", written_units.number.to_string()),
                    });
                }
                // An augmentation, or a reduction the lot covers, keeps the lot's identity.
                // A partial cover of a short still has a negative balance (#614).
                self.update_lot(&account, &target_lot_record, &(&target_lot_record.amount).add(&accr_amount));
                lot_weights.push(lot_weight(&target_lot_record, accr_amount.clone()));
                legs.push(leg(&target_lot_record, accr_amount, &cost, &units.commodity));
                break;
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
        if weighs_by_lots(posting) && legs.iter().any(|leg| leg.cost.as_ref().is_some_and(|cost| cost.base.is_none())) {
            if !errors.iter().any(|error| error.kind == ErrorKind::NoEnoughCommodityLot) {
                errors.push(BookingError {
                    kind: ErrorKind::NoEnoughCommodityLot,
                    metas: HashMap::of("transaction_amount", written_units.number.to_string()),
                });
            }
            return Err(errors);
        }
        Ok(PostingBooking {
            weight,
            errors,
            legs: merge_legs(legs),
        })
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

    /// the lots of every account the fold booked a posting on, in lot order
    pub(crate) fn into_lots(self) -> HashMap<String, Vec<CommodityLotRecord>> {
        self.lots
    }

    /// Whether the account's own booked units are nonzero in any commodity, where the store
    /// fold stands. Lots at different costs or with different labels can cancel in units.
    pub(crate) fn has_non_zero_balance(&self, account: &str) -> bool {
        let mut units: HashMap<&str, BigDecimal> = HashMap::new();
        for lot in self.lots.get(account).into_iter().flatten() {
            *units.entry(&lot.commodity).or_default() += &lot.amount;
        }
        units.values().any(|number| !number.is_zero())
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
            units: &units.number,
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

        let lot_record = pick(matching_lots(entry, filter), booking_method, filter.units).cloned();
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

/// the lot `booking_method` books against first among `lots`: opposite-sign lots first, then the
/// oldest acquisition date, or the newest for LIFO (E10). Lots of the same date go in
/// creation order, reversed for LIFO, so LIFO takes the one created last: the order is exactly
/// FIFO's, reversed. STRICT books like FIFO once `ambiguous_reduction` has checked the match. NONE,
/// AVERAGE and AVERAGE_ONLY never get here: they resolve to the default method at the `open`
fn pick<'a>(lots: impl Iterator<Item = &'a CommodityLotRecord>, booking_method: BookingMethod, units: &BigDecimal) -> Option<&'a CommodityLotRecord> {
    match booking_method {
        // the last of the equally newest
        BookingMethod::Lifo => lots.max_by_key(|lot| (reduces(lot, units), lot.acquisition_date)),
        // the first of the equally oldest
        BookingMethod::Fifo | BookingMethod::Strict | BookingMethod::Average | BookingMethod::AverageOnly | BookingMethod::None => {
            lots.min_by_key(|lot| (!reduces(lot, units), lot.acquisition_date))
        }
    }
}

/// whether booking `units` against `lot` reduces it: they have opposite signs
fn reduces(lot: &CommodityLotRecord, units: &BigDecimal) -> bool {
    (lot.amount.is_positive() && units.is_negative()) || (lot.amount.is_negative() && units.is_positive())
}

/// one [`ErrorKind::CostMergingNotSupported`] per posting written with the merge-cost marker
/// (`{*}`), as beancount reports it: the legs of a booked posting carry the lot they book against,
/// so the marker is read from the written form they share
fn merge_cost_errors(postings: &[Posting]) -> Vec<BookingError> {
    written_groups(postings)
        .iter()
        .filter(|group| {
            let cost = match group.written {
                Some(written) => written.cost.as_ref(),
                None => group.legs[0].cost.as_ref(),
            };
            cost.is_some_and(|cost| cost.merge)
        })
        .map(|_| BookingError {
            kind: ErrorKind::CostMergingNotSupported,
            metas: HashMap::new(),
        })
        .collect()
}

/// whether the posting's weight is decided by the lots it books against: an explicit posting with
/// a cost but no cost number (`{}`, `{{}}`, `{date}`). Any other posting weighs as written
pub(crate) fn weighs_by_lots(posting: &Posting) -> bool {
    posting.units.is_some() && posting.cost.as_ref().is_some_and(|cost| cost.base.is_none())
}

/// the weight of `units` booked against `lot`: `units × lot cost` in the cost's commodity, in the
/// 28-digit decimal context like beancount's weights, or the units themselves for a lot held
/// without cost
fn lot_weight(lot: &CommodityLotRecord, units: BigDecimal) -> Amount {
    match &lot.cost {
        Some(cost) => Amount::new(mul_in_context(&units, &cost.number), cost.commodity.clone()),
        None => Amount::new(units, lot.commodity.clone()),
    }
}

/// the leg of a posting booked against `lot`: `units` of `commodity` at the lot's cost, acquisition
/// date and label. A lot without a cost gives the leg the cost spec as `written`; an unresolved
/// cost leg then rejects the transaction, and its tentative lot changes are rolled back (E9)
fn leg(lot: &CommodityLotRecord, units: BigDecimal, written: &PostingCost, commodity: &str) -> Leg {
    let cost = match &lot.cost {
        Some(cost) => PostingCost {
            base: Some(cost.clone()),
            date: lot.acquisition_date.map(Date::Date),
            label: lot.label.clone(),
            ..PostingCost::default()
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

/// whether every posting of the transaction is booked, structurally (design §5.2): it has units,
/// and its cost, if any, has a number and a date. The written form plays no part, so a plugin that
/// drops it changes nothing here. A rejected transaction with an unresolved cost is not booked
pub(crate) fn is_booked(txn: &Transaction) -> bool {
    txn.postings
        .iter()
        .all(|posting| posting.units.is_some() && posting.cost.as_ref().is_none_or(|cost| cost.base.is_some() && cost.date.is_some()))
}

/// the most decimals written in the transaction per commodity: of the units, and of the cost and
/// price numbers, per-unit or total, of every posting. `@ 7.12345 CNY` counts 5 for CNY and
/// `{{1000 USD}}` 0 for USD. A booked leg counts what its posting was written as: the lot cost it
/// carries may be a long division (`{{100 CNY}}` over 3 units, 28 significant digits), which is
/// not a written scale
fn written_scales<'a>(postings: &[TxnPosting<'a>]) -> HashMap<&'a str, i64> {
    let mut scales: HashMap<&str, i64> = HashMap::new();
    for posting in postings {
        let posting = posting.posting;
        let (units, cost) = match &posting.written {
            Some(written) => (written.units.as_ref(), written.cost.as_ref()),
            None => (posting.units.as_ref(), posting.cost.as_ref()),
        };
        let base = cost.and_then(|cost| cost.base.as_ref());
        let price = posting.price.as_ref().map(|price| match price {
            SingleTotalPrice::Single(amount) | SingleTotalPrice::Total(amount) => amount,
        });
        for amount in units.into_iter().chain(base).chain(price) {
            let scale = scales.entry(amount.commodity.as_str()).or_insert(0);
            *scale = (*scale).max(amount.number.fractional_digit_count());
        }
        // the total part of a compound cost (`{100 # 5.25 USD}`) is written in the cost's commodity
        if let (Some(base), Some(total)) = (base, cost.and_then(|cost| cost.compound_total.as_ref())) {
            let scale = scales.entry(base.commodity.as_str()).or_insert(0);
            *scale = (*scale).max(total.fractional_digit_count());
        }
    }
    scales
}

/// `number` without its division dust. A sum of exact weights (not `rounded`) with at most
/// [`EXACT_DECIMALS`] decimals is exact and kept as is. Any other is rounded at `scale` with its
/// rounding, like the balance check rounds a residual, or kept as is without a scale (an undefined
/// commodity not written in the transaction). A trimmed number drops its trailing zeros (`1000.00`
/// is `1000`)
fn round(number: &BigDecimal, scale: Option<(i64, RoundingMode)>, rounded: bool) -> BigDecimal {
    if !rounded && number.fractional_digit_count() <= EXACT_DECIMALS {
        return number.clone();
    }
    let normalized = without_trailing_zeros(number);
    if !rounded && normalized.fractional_digit_count() <= EXACT_DECIMALS {
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

/// the weights of the explicit postings of a transaction, summed per commodity, which the implicit
/// posting or a missing cost balances
#[derive(Default)]
struct Weights {
    sums: BTreeMap<Currency, BigDecimal>,
    /// the commodities with a weight at the full precision of the decimal context
    /// ([`DIVISION_PRECISION`] significant digits): units at a cost divided from a total or
    /// inferred from the other postings. The last digits of such a weight are the rounding of the
    /// division, not written decimals, and with a large cost they fall within [`EXACT_DECIMALS`]
    rounded: HashSet<Currency>,
}

impl Weights {
    fn add(&mut self, weight: impl IntoIterator<Item = Amount>) {
        for amount in weight {
            if amount.number.digits() >= DIVISION_PRECISION {
                self.rounded.insert(amount.commodity.clone());
            }
            self.sums.entry(amount.commodity).or_insert_with(BigDecimal::zero).add_assign(amount.number);
        }
    }

    /// `number`, a sum of the weights in `commodity`, without its division dust ([`round`])
    fn round(&self, commodity: &str, number: &BigDecimal, scale: Option<(i64, RoundingMode)>) -> BigDecimal {
        round(number, scale, self.rounded.contains(commodity))
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
pub(crate) mod tests;
