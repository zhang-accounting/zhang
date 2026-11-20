//! Generated ledgers for the budget baseline of #698 (P0, `zhang-server/tests/budget_baseline.rs`): a household's
//! ledger of a given number of transactions over a number of months, with a given number of budgets, and the same
//! ledger without any budget directive or `budget` metadata.
//!
//! The ledger exercises every part of the native budget code:
//!
//! - expense accounts in the categories of a household, linked to budgets by the `budget` metadata of their
//!   `open`s: `Expenses:Food:Market` is in two budgets, and two accounts are closed and opened again with other
//!   budgets (`Expenses:Transport:Taxi` moves from `Taxi` to `Transit`, `Expenses:Food:Coffee` from `Dining` to
//!   `Dining` and `Fun-Money`); a few expense accounts belong to no budget;
//! - USD as the operating currency, budgets in EUR, JPY and CNY as well, postings in EUR, GBP, JPY and CNY next to
//!   the USD ones (some through an `@` price), a few postings of `GOLD` held at cost, and the monthly prices that
//!   convert all of them to the budgets' commodities (directly, through an inverse rate, or through the cost
//!   currency), so that no posting is left out;
//! - a `budget-add` per open budget and month (some in another commodity than the budget's), two to four
//!   `budget-transfer`s a month (some between commodities), a budget defined after the start, and four
//!   `budget-close`s (one with a time), each followed by a transfer of what is left;
//! - the routine of a household around it: salaries, rent, insurance, a subscription, card payments, currency
//!   top-ups and a monthly purchase of shares at cost.
//!
//! It loads without any budget error. Its only errors are the `CloseNonZeroAccount` of the two reopened accounts: an
//! expense account has a balance when it closes.
//!
//! The files are a function of the [`Spec`] alone, byte for byte, on any platform: [`GeneratedLedger::sha256`] is
//! the digest the baseline page records. The transactions and prices do not depend on the budgets: a spec, the same
//! spec [`Spec::without_budgets`] and the same spec with another number of budgets give the same ledger but for the
//! budget directives and the `budget` metadata.

// amounts and rates are written with an underscore before their decimals: `600_00` cents are 600.00, `1_2200` is
// 1.2200 in units of 1e-4
#![allow(clippy::inconsistent_digit_grouping)]

use std::fmt::Write as _;
use std::path::Path;

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use sha2::{Digest, Sha256};
use zhang_core::clock::Clock;

use crate::ledger::Scratch;
use crate::XorShift;

/// the seed of the baseline's ledgers
pub const SEED: u64 = 698;
/// the `timezone` option of the generated ledgers
pub const TIMEZONE: &str = "Asia/Shanghai";
/// the main file of a generated ledger
pub const MAIN: &str = "main.zhang";
/// how many budgets a ledger can have: the budgets of the household, in the order a spec takes them
pub const MAX_BUDGETS: usize = BUDGETS.len();

/// What to generate. The ledger starts on 2021-01-01 and has a file per year besides its main file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    pub seed: u64,
    /// how many transactions, the routine ones included (there are at least the 10 a month of the routine, and 2)
    pub transactions: usize,
    /// how many budgets, the first of the household's ([`MAX_BUDGETS`] at most)
    pub budgets: usize,
    /// how many months, from January 2021 (at least 6)
    pub months: u32,
    /// false for the same ledger without the budget directives and the `budget` metadata
    pub with_budgets: bool,
}

impl Spec {
    /// `transactions` transactions and `budgets` budgets over 60 months, with the baseline's seed.
    pub fn new(transactions: usize, budgets: usize) -> Spec {
        assert!(budgets <= MAX_BUDGETS, "at most {MAX_BUDGETS} budgets");
        Spec {
            seed: SEED,
            transactions,
            budgets,
            months: 60,
            with_budgets: true,
        }
    }

    /// the spec over `months` months
    pub fn with_months(self, months: u32) -> Spec {
        assert!((6..=1200).contains(&months), "6 to 1200 months");
        Spec { months, ..self }
    }

    /// the spec with another seed
    pub fn with_seed(self, seed: u64) -> Spec {
        Spec { seed, ..self }
    }

    /// the same ledger without budget directives and `budget` metadata
    pub fn without_budgets(self) -> Spec {
        Spec { with_budgets: false, ..self }
    }

    /// a name for the spec, such as `10000tx-30b-60m` or `10000tx-60m-no-budgets`
    pub fn name(&self) -> String {
        match self.with_budgets {
            true => format!("{}tx-{}b-{}m", self.transactions, self.budgets, self.months),
            false => format!("{}tx-{}m-no-budgets", self.transactions, self.months),
        }
    }

    /// the first day of the ledger, 2021-01-01
    pub fn first_day(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(2021, 1, 1).expect("a date")
    }

    /// the first day of the ledger's last month, the current month of [`Spec::clock`]
    pub fn current_month(&self) -> NaiveDate {
        self.month(self.months - 1)
    }

    /// the last day of the ledger
    pub fn last_day(&self) -> NaiveDate {
        self.month(self.months).pred_opt().expect("a date")
    }

    /// The clock to load the ledger with: noon of its last day in [`TIMEZONE`], so that `today()` is that day
    /// whatever the date is.
    pub fn clock(&self) -> Clock {
        let noon = self.last_day().and_hms_opt(4, 0, 0).expect("a time");
        Clock::Fixed(Utc.from_utc_datetime(&noon))
    }

    /// the first day of month `index` of the ledger, 0 being January 2021
    fn month(&self, index: u32) -> NaiveDate {
        self.first_day().checked_add_months(Months::new(index)).expect("a date")
    }

    /// The ledger's files.
    pub fn generate(&self) -> GeneratedLedger {
        Generator::new(self).run()
    }
}

/// The files of a generated ledger, ordered by path: a file per year (`2021.zhang`, ...) and [`MAIN`], which includes
/// them.
pub struct GeneratedLedger {
    files: Vec<(String, String)>,
}

impl GeneratedLedger {
    /// the files: their paths, relative to the ledger's directory, and their texts, ordered by path
    pub fn files(&self) -> &[(String, String)] {
        &self.files
    }

    /// the size of the files, in bytes
    pub fn bytes(&self) -> usize {
        self.files.iter().map(|(_, text)| text.len()).sum()
    }

    /// The files in a scratch directory, removed when it is dropped.
    pub fn scratch(&self) -> Scratch {
        let files = self.files.iter().map(|(path, text)| (path.as_str(), text.as_str())).collect::<Vec<_>>();
        Scratch::with_files(MAIN, &files)
    }

    /// Write the files into `dir`, which is created when missing.
    pub fn write_to(&self, dir: &Path) {
        std::fs::create_dir_all(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display()));
        for (path, text) in &self.files {
            let path = dir.join(path);
            std::fs::write(&path, text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
    }

    /// The SHA-256 of the `sha256sum` listing of the files, in path order: what
    /// `cd <dir> && LC_ALL=C sha256sum *.zhang | sha256sum` prints for a directory the files were written to.
    pub fn sha256(&self) -> String {
        let listing = self
            .files
            .iter()
            .map(|(path, text)| format!("{}  {}\n", hex(&Sha256::digest(text.as_bytes())), path))
            .collect::<String>();
        hex(&Sha256::digest(listing.as_bytes()))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---- the household ----------------------------------------------------------------------------------------------

/// A budget of the household.
struct BudgetDef {
    name: &'static str,
    commodity: Commodity,
    category: &'static str,
    alias: Option<&'static str>,
    /// what is assigned to it each month, in the smallest unit of its commodity
    monthly: i64,
    /// the accounts whose first `open` links them to it
    accounts: &'static [&'static str],
}

const fn budget(
    name: &'static str, commodity: Commodity, category: &'static str, alias: Option<&'static str>, monthly: i64, accounts: &'static [&'static str],
) -> BudgetDef {
    BudgetDef {
        name,
        commodity,
        category,
        alias,
        monthly,
        accounts,
    }
}

use Commodity::{Cny, Eur, Jpy, Usd};

/// The budgets, in the order a spec takes them: the first 30 have every case the module docs list.
const BUDGETS: &[BudgetDef] = &[
    budget("Groceries", Usd, "Living", None, 600_00, &["Expenses:Food:Groceries", "Expenses:Food:Market"]),
    budget(
        "Dining",
        Usd,
        "Living",
        Some("Eating out"),
        350_00,
        &["Expenses:Food:Restaurants", "Expenses:Food:Coffee", "Expenses:Food:Delivery"],
    ),
    budget("Household", Usd, "Living", None, 120_00, &["Expenses:Home:Supplies", "Expenses:Food:Market"]),
    budget("Rent", Usd, "Living", None, 1800_00, &["Expenses:Home:Rent"]),
    budget(
        "Utilities",
        Usd,
        "Living",
        None,
        300_00,
        &["Expenses:Home:Electricity", "Expenses:Home:Water", "Expenses:Home:Gas"],
    ),
    budget(
        "Internet",
        Usd,
        "Living",
        Some("Internet and phone"),
        120_00,
        &["Expenses:Home:Internet", "Expenses:Home:Phone"],
    ),
    budget("Transit", Usd, "Transport", None, 120_00, &["Expenses:Transport:Transit"]),
    budget(
        "Car",
        Usd,
        "Transport",
        None,
        300_00,
        &[
            "Expenses:Transport:Fuel",
            "Expenses:Transport:Parking",
            "Expenses:Transport:Service",
            "Expenses:Insurance:Car",
        ],
    ),
    budget("Taxi", Usd, "Transport", None, 80_00, &["Expenses:Transport:Taxi"]),
    budget("Medical", Usd, "Health", None, 150_00, &["Expenses:Health:Doctor", "Expenses:Health:Pharmacy"]),
    budget("Gym", Usd, "Health", None, 50_00, &["Expenses:Health:Gym"]),
    budget("Insurance", Usd, "Health", None, 250_00, &["Expenses:Insurance:Health"]),
    budget("Clothing", Usd, "Personal", None, 150_00, &["Expenses:Shopping:Clothing"]),
    budget("Electronics", Usd, "Personal", None, 100_00, &["Expenses:Shopping:Electronics"]),
    budget(
        "Personal-Care",
        Usd,
        "Personal",
        Some("Personal care"),
        80_00,
        &["Expenses:Personal:Haircut", "Expenses:Personal:Cosmetics"],
    ),
    budget("Books", Usd, "Personal", None, 40_00, &["Expenses:Education:Books"]),
    budget("Courses", Usd, "Personal", None, 100_00, &["Expenses:Education:Courses"]),
    budget(
        "Subscriptions",
        Usd,
        "Personal",
        None,
        60_00,
        &["Expenses:Fun:Streaming", "Expenses:Fun:Software"],
    ),
    budget(
        "Entertainment",
        Usd,
        "Fun",
        None,
        120_00,
        &["Expenses:Fun:Cinema", "Expenses:Fun:Concerts", "Expenses:Fun:Games"],
    ),
    budget("Fun-Money", Usd, "Fun", Some("Fun money"), 100_00, &["Expenses:Fun:Hobbies"]),
    budget(
        "Travel-Europe",
        Eur,
        "Travel",
        Some("Europe trips"),
        300_00,
        &["Expenses:Travel:Europe:Hotels", "Expenses:Travel:Europe:Food", "Expenses:Travel:Souvenirs"],
    ),
    budget("Flights", Usd, "Travel", None, 200_00, &["Expenses:Travel:Flights"]),
    budget("Japan", Jpy, "Travel", Some("Japan trips"), 30_000, &["Expenses:Travel:Japan"]),
    budget("Kids", Usd, "Family", None, 400_00, &["Expenses:Family:Kids", "Expenses:Family:School"]),
    budget("Pets", Usd, "Family", None, 90_00, &["Expenses:Family:Pets"]),
    budget("Gifts", Usd, "Family", None, 100_00, &["Expenses:Gifts"]),
    budget("Charity", Usd, "Family", None, 50_00, &["Expenses:Charity"]),
    budget("Remittance", Cny, "Family", Some("Parents"), 3000_00, &["Expenses:Family:Remittance"]),
    budget("Wedding", Usd, "Goals", None, 1500_00, &["Expenses:Goals:Wedding"]),
    budget("New-Laptop", Usd, "Goals", Some("New laptop"), 100_00, &["Expenses:Goals:Laptop"]),
    budget("Home-Repair", Usd, "Living", None, 150_00, &["Expenses:Home:Repairs"]),
    budget("Emergency", Usd, "Goals", Some("Emergency fund"), 200_00, &[]),
];

/// The budgets an account reopened in the middle of the ledger links to from then on.
const REOPENED: &[(&str, &[&str])] = &[("Expenses:Transport:Taxi", &["Transit"]), ("Expenses:Food:Coffee", &["Dining", "Fun-Money"])];

/// An account the random transactions spend from.
struct Spending {
    account: &'static str,
    /// how often a transaction books to it, relative to the others
    weight: u64,
    /// the range of an amount, in US cents (an amount in another currency is about as much)
    range: (i64, i64),
    mix: Mix,
    /// when it takes postings
    window: Window,
    payees: &'static [&'static str],
    narration: &'static str,
}

/// the currencies a spending account's postings are in
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mix {
    /// mostly USD, some EUR and GBP
    Usd,
    /// mostly EUR, some USD
    Europe,
    /// mostly EUR, some GOLD at cost
    Souvenirs,
    /// mostly JPY, some USD
    Japan,
    /// mostly CNY, some USD
    China,
    /// mostly USD, some GOLD at cost
    Gifts,
    /// USD, EUR and GBP
    Flights,
}

/// when an account takes postings
#[derive(Clone, Copy, PartialEq, Eq)]
enum Window {
    Always,
    /// until the day before the `Gym` budget closes
    Gym,
    /// until the day before the `New-Laptop` budget closes
    Laptop,
    /// from the day after the `Wedding` budget is defined until the day before it closes
    Wedding,
}

const fn spending(account: &'static str, weight: u64, range: (i64, i64), mix: Mix, payees: &'static [&'static str], narration: &'static str) -> Spending {
    Spending {
        account,
        weight,
        range,
        mix,
        window: Window::Always,
        payees,
        narration,
    }
}

const fn spending_in(window: Window, spending: Spending) -> Spending {
    Spending { window, ..spending }
}

const SPENDING: &[Spending] = &[
    spending(
        "Expenses:Food:Groceries",
        14,
        (15_00, 180_00),
        Mix::Usd,
        &["Whole Foods", "Trader Joe's", "Safeway", "Costco"],
        "groceries",
    ),
    spending(
        "Expenses:Food:Market",
        6,
        (5_00, 60_00),
        Mix::Usd,
        &["Farmers Market", "Corner Store"],
        "market",
    ),
    spending(
        "Expenses:Food:Restaurants",
        9,
        (15_00, 120_00),
        Mix::Usd,
        &["Chez Panisse", "Sushi Ran", "Taqueria", "Pho 24"],
        "dinner",
    ),
    spending(
        "Expenses:Food:Coffee",
        8,
        (3_00, 9_00),
        Mix::Usd,
        &["Blue Bottle", "Starbucks", "Philz"],
        "coffee",
    ),
    spending("Expenses:Food:Delivery", 5, (15_00, 60_00), Mix::Usd, &["DoorDash", "Uber Eats"], "delivery"),
    spending(
        "Expenses:Home:Supplies",
        4,
        (5_00, 50_00),
        Mix::Usd,
        &["Target", "IKEA", "Home Depot"],
        "supplies",
    ),
    spending("Expenses:Home:Electricity", 1, (40_00, 150_00), Mix::Usd, &["PG&E"], "electricity"),
    spending("Expenses:Home:Water", 1, (20_00, 60_00), Mix::Usd, &["City Water"], "water"),
    spending("Expenses:Home:Gas", 1, (20_00, 90_00), Mix::Usd, &["PG&E"], "gas"),
    spending("Expenses:Home:Internet", 1, (50_00, 80_00), Mix::Usd, &["Comcast"], "internet"),
    spending("Expenses:Home:Phone", 1, (30_00, 70_00), Mix::Usd, &["T-Mobile"], "phone"),
    spending("Expenses:Home:Repairs", 1, (30_00, 400_00), Mix::Usd, &["Handyman", "Home Depot"], "repairs"),
    spending(
        "Expenses:Transport:Transit",
        7,
        (2_00, 15_00),
        Mix::Usd,
        &["BART", "Muni", "Caltrain"],
        "transit",
    ),
    spending("Expenses:Transport:Taxi", 3, (8_00, 45_00), Mix::Usd, &["Uber", "Lyft"], "ride"),
    spending("Expenses:Transport:Fuel", 4, (25_00, 70_00), Mix::Usd, &["Shell", "Chevron"], "fuel"),
    spending("Expenses:Transport:Parking", 3, (3_00, 25_00), Mix::Usd, &["ParkMobile", "SFMTA"], "parking"),
    spending("Expenses:Transport:Service", 1, (80_00, 600_00), Mix::Usd, &["Toyota Service"], "car service"),
    spending("Expenses:Insurance:Car", 1, (90_00, 150_00), Mix::Usd, &["Geico"], "car insurance"),
    spending("Expenses:Health:Doctor", 1, (30_00, 250_00), Mix::Usd, &["Kaiser"], "doctor"),
    spending("Expenses:Health:Pharmacy", 2, (5_00, 60_00), Mix::Usd, &["CVS", "Walgreens"], "pharmacy"),
    spending_in(Window::Gym, spending("Expenses:Health:Gym", 1, (20_00, 60_00), Mix::Usd, &["Equinox"], "gym")),
    spending(
        "Expenses:Shopping:Clothing",
        3,
        (20_00, 200_00),
        Mix::Usd,
        &["Uniqlo", "Zara", "REI"],
        "clothes",
    ),
    spending(
        "Expenses:Shopping:Electronics",
        1,
        (20_00, 500_00),
        Mix::Usd,
        &["Apple", "Best Buy"],
        "electronics",
    ),
    spending("Expenses:Personal:Haircut", 1, (20_00, 60_00), Mix::Usd, &["Barber"], "haircut"),
    spending("Expenses:Personal:Cosmetics", 2, (8_00, 60_00), Mix::Usd, &["Sephora"], "cosmetics"),
    spending(
        "Expenses:Education:Books",
        2,
        (8_00, 40_00),
        Mix::Usd,
        &["Green Apple Books", "Amazon"],
        "books",
    ),
    spending("Expenses:Education:Courses", 1, (20_00, 300_00), Mix::Usd, &["Coursera", "Udemy"], "course"),
    spending("Expenses:Fun:Software", 1, (5_00, 30_00), Mix::Usd, &["App Store", "JetBrains"], "software"),
    spending("Expenses:Fun:Cinema", 2, (10_00, 40_00), Mix::Usd, &["AMC"], "movie"),
    spending("Expenses:Fun:Concerts", 1, (40_00, 200_00), Mix::Usd, &["Ticketmaster"], "concert"),
    spending("Expenses:Fun:Games", 2, (5_00, 70_00), Mix::Usd, &["Steam", "Nintendo"], "games"),
    spending("Expenses:Fun:Hobbies", 3, (10_00, 150_00), Mix::Usd, &["Blick Art", "Guitar Center"], "hobby"),
    spending(
        "Expenses:Travel:Europe:Hotels",
        1,
        (80_00, 300_00),
        Mix::Europe,
        &["Hotel Adlon", "Ibis", "Airbnb"],
        "hotel",
    ),
    spending(
        "Expenses:Travel:Europe:Food",
        2,
        (10_00, 80_00),
        Mix::Europe,
        &["Bistro", "Trattoria", "Biergarten"],
        "meal abroad",
    ),
    spending("Expenses:Travel:Souvenirs", 1, (5_00, 50_00), Mix::Souvenirs, &["Gift Shop"], "souvenir"),
    spending(
        "Expenses:Travel:Flights",
        1,
        (150_00, 1200_00),
        Mix::Flights,
        &["United", "Lufthansa", "British Airways"],
        "flight",
    ),
    spending(
        "Expenses:Travel:Japan",
        2,
        (5_00, 150_00),
        Mix::Japan,
        &["Lawson", "Ichiran", "JR East"],
        "japan",
    ),
    spending("Expenses:Family:Kids", 3, (10_00, 150_00), Mix::Usd, &["Toy Store", "Kids Club"], "kids"),
    spending("Expenses:Family:School", 1, (50_00, 500_00), Mix::Usd, &["School District"], "school"),
    spending("Expenses:Family:Pets", 2, (10_00, 120_00), Mix::Usd, &["Petco", "Vet Clinic"], "pets"),
    spending("Expenses:Family:Remittance", 1, (200_00, 1000_00), Mix::China, &["Parents"], "remittance"),
    spending("Expenses:Gifts", 2, (20_00, 200_00), Mix::Gifts, &["Etsy", "Tiffany"], "gift"),
    spending("Expenses:Charity", 1, (10_00, 100_00), Mix::Usd, &["Red Cross", "EFF"], "donation"),
    spending_in(
        Window::Wedding,
        spending(
            "Expenses:Goals:Wedding",
            1,
            (200_00, 5000_00),
            Mix::Usd,
            &["Venue", "Florist", "Caterer"],
            "wedding",
        ),
    ),
    spending_in(
        Window::Laptop,
        spending("Expenses:Goals:Laptop", 1, (20_00, 200_00), Mix::Usd, &["Apple"], "laptop accessories"),
    ),
    spending("Expenses:Fees:Bank", 1, (2_00, 35_00), Mix::Usd, &["Bank"], "fee"),
    spending("Expenses:Misc", 2, (3_00, 50_00), Mix::Usd, &["Shop"], "misc"),
];

/// The accounts besides the spending ones, opened on the first day, with their currency constraint.
const OTHER_ACCOUNTS: &[(&str, &str)] = &[
    ("Assets:Bank:Checking", "USD"),
    ("Assets:Bank:EUR", "EUR"),
    ("Assets:Bank:CNY", "CNY"),
    ("Assets:Cash", "USD"),
    ("Assets:Cash:JPY", "JPY"),
    ("Assets:Brokerage", ""),
    ("Liabilities:CreditCard", "USD"),
    ("Income:Salary", "USD"),
    ("Equity:Opening-Balances", ""),
    ("Expenses:Home:Rent", ""),
    ("Expenses:Insurance:Health", ""),
    ("Expenses:Fun:Streaming", ""),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Commodity {
    Usd,
    Eur,
    Gbp,
    Jpy,
    Cny,
}

impl Commodity {
    fn code(self) -> &'static str {
        match self {
            Commodity::Usd => "USD",
            Commodity::Eur => "EUR",
            Commodity::Gbp => "GBP",
            Commodity::Jpy => "JPY",
            Commodity::Cny => "CNY",
        }
    }

    /// the decimals of an amount
    fn decimals(self) -> u32 {
        match self {
            Commodity::Jpy => 0,
            _ => 2,
        }
    }

    /// about as much as `cents` US cents, in the smallest unit of the commodity
    fn units_of_cents(self, cents: i64) -> i64 {
        let per_hundred = match self {
            Commodity::Usd => 100,
            Commodity::Eur => 92,
            Commodity::Gbp => 79,
            Commodity::Jpy => 145,
            Commodity::Cny => 710,
        };
        (cents * per_hundred / 100).max(1)
    }

    /// `units` of the smallest unit, as written
    fn amount(self, units: i64) -> String {
        format!("{} {}", decimal(units, self.decimals()), self.code())
    }
}

/// `units` with `decimals` decimals: `decimal(12345, 2)` is `123.45`
fn decimal(units: i64, decimals: u32) -> String {
    if decimals == 0 {
        return units.to_string();
    }
    let scale = 10i64.pow(decimals);
    let sign = if units < 0 { "-" } else { "" };
    let units = units.abs();
    format!("{sign}{}.{:0width$}", units / scale, units % scale, width = decimals as usize)
}

/// The prices of a month: the rates of its first day, and of EUR on its 15th.
struct Prices {
    /// USD per EUR, in 1e-4
    eur: i64,
    eur_mid: i64,
    /// USD per GBP, in 1e-4
    gbp: i64,
    /// USD per JPY, in 1e-6
    jpy: i64,
    /// CNY per USD, in 1e-4
    cny: i64,
    /// USD per gram of GOLD, in cents
    gold: i64,
    /// USD per share of VTI, in cents
    vti: i64,
}

/// the months, from the first, of what happens to the budgets with a life of their own
struct Plan {
    wedding_from: u32,
    wedding_close: u32,
    laptop_close: u32,
    gym_close: u32,
    taxi_reopen: u32,
    coffee_reopen: u32,
}

/// the order of the directives of a day in the files
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Open,
    Price,
    Budget,
    BudgetAdd,
    BudgetTransfer,
    Transaction,
    BudgetClose,
    Close,
}

struct Event {
    date: NaiveDate,
    class: Class,
    /// the seconds of its time of day, 0 when it has none
    seconds: u32,
    text: String,
}

struct Generator<'s> {
    spec: &'s Spec,
    plan: Plan,
    prices: Vec<Prices>,
    events: Vec<Event>,
}

/// the independent random streams of a seed: the prices, the transactions and the budgets
fn stream(seed: u64, stream: u64) -> XorShift {
    let mixed = (seed ^ stream.wrapping_mul(0xA076_1D64_78BD_642F)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    XorShift(mixed.max(1))
}

impl<'s> Generator<'s> {
    fn new(spec: &'s Spec) -> Generator<'s> {
        let months = spec.months;
        Generator {
            spec,
            plan: Plan {
                wedding_from: (months / 4).max(1),
                wedding_close: months * 7 / 10,
                laptop_close: months * 2 / 5,
                gym_close: months / 2,
                taxi_reopen: months * 3 / 5,
                coffee_reopen: months * 4 / 5,
            },
            prices: vec![],
            events: vec![],
        }
    }

    fn day(&self, month: u32, day: u32) -> NaiveDate {
        self.spec.month(month).with_day(day).expect("a day every month has")
    }

    fn push(&mut self, date: NaiveDate, class: Class, time: Option<NaiveTime>, text: String) {
        let seconds = time.map(|it| it.num_seconds_from_midnight()).unwrap_or(0);
        self.events.push(Event { date, class, seconds, text });
    }

    /// the date, with its time when it has one
    fn stamp(date: NaiveDate, time: Option<NaiveTime>) -> String {
        match time {
            Some(time) => format!("{} {}", date, time.format("%H:%M:%S")),
            None => date.to_string(),
        }
    }

    fn run(mut self) -> GeneratedLedger {
        self.prices();
        self.routine();
        self.random_transactions();
        self.later_opens();
        if self.spec.with_budgets {
            self.budget_directives();
        }
        self.files()
    }

    /// whether a spec's budget `name` is in the ledger
    fn selected(&self, name: &str) -> bool {
        self.spec.with_budgets && BUDGETS[..self.spec.budgets].iter().any(|it| it.name == name)
    }

    // ---- prices -------------------------------------------------------------------------------------------------

    fn prices(&mut self) {
        let mut rng = stream(self.spec.seed, 1);
        // a random walk of up to 2% a step
        let mut step = |value: &mut i64| {
            *value += *value * (rng.below(401) as i64 - 200) / 10_000;
        };
        let (mut eur, mut gbp, mut jpy, mut cny, mut gold, mut vti) = (1_2200, 1_3700, 9_600, 6_4500, 60_00, 195_00);
        for month in 0..self.spec.months {
            if month > 0 {
                for value in [&mut eur, &mut gbp, &mut jpy, &mut cny, &mut gold, &mut vti] {
                    step(value);
                }
            }
            let mut eur_mid = eur;
            step(&mut eur_mid);
            self.prices.push(Prices {
                eur,
                eur_mid,
                gbp,
                jpy,
                cny,
                gold,
                vti,
            });
        }
        for month in 0..self.spec.months {
            let (first, mid) = (self.day(month, 1), self.day(month, 15));
            let p = &self.prices[month as usize];
            let lines = [
                (first, format!("{first} price EUR {} USD\n", decimal(p.eur, 4))),
                (first, format!("{first} price GBP {} USD\n", decimal(p.gbp, 4))),
                (first, format!("{first} price JPY {} USD\n", decimal(p.jpy, 6))),
                (first, format!("{first} price USD {} CNY\n", decimal(p.cny, 4))),
                (first, format!("{first} price GOLD {} USD\n", decimal(p.gold, 2))),
                (first, format!("{first} price VTI {} USD\n", decimal(p.vti, 2))),
                (mid, format!("{mid} price EUR {} USD\n", decimal(p.eur_mid, 4))),
            ];
            for (date, text) in lines {
                self.push(date, Class::Price, None, text);
            }
        }
    }

    /// the prices of the month of `date`
    fn prices_at(&self, date: NaiveDate) -> &Prices {
        let first = self.spec.first_day();
        let month = (date.year() - first.year()) * 12 + date.month0() as i32;
        &self.prices[month as usize]
    }

    // ---- transactions -------------------------------------------------------------------------------------------

    fn transaction(&mut self, date: NaiveDate, time: Option<NaiveTime>, payee: &str, narration: &str, postings: &[String]) {
        let mut text = format!("{} * \"{}\" \"{}\"\n", Self::stamp(date, time), payee, narration);
        for posting in postings {
            text.push_str("  ");
            text.push_str(posting);
            text.push('\n');
        }
        self.push(date, Class::Transaction, time, text);
    }

    /// how many transactions the routine has
    fn routine_count(&self) -> usize {
        2 + 10 * self.spec.months as usize
    }

    /// the transactions of every month: salaries, rent, top-ups of the foreign accounts, a subscription, shares,
    /// insurance and the card payment; the opening balance and the purchase of the laptop
    fn routine(&mut self) {
        let mut rng = stream(self.spec.seed, 2);
        let first = self.spec.first_day();
        self.transaction(
            first,
            None,
            "Bank",
            "opening balance",
            &["Assets:Bank:Checking 20000.00 USD".to_owned(), "Equity:Opening-Balances".to_owned()],
        );
        let laptop = self.day(self.plan.laptop_close, 10);
        self.transaction(
            laptop,
            None,
            "Apple",
            "new laptop",
            &["Expenses:Goals:Laptop 1899.00 USD".to_owned(), "Liabilities:CreditCard".to_owned()],
        );
        let checking = "Assets:Bank:Checking".to_owned();
        for month in 0..self.spec.months {
            let p = &self.prices[month as usize];
            let (eur, jpy, cny, vti) = (p.eur, p.jpy, p.cny, p.vti);
            for day in [1, 15] {
                let salary = ["Assets:Bank:Checking 4200.00 USD".to_owned(), "Income:Salary".to_owned()];
                self.transaction(self.day(month, day), None, "Acme Corp", "salary", &salary);
            }
            let rent = ["Expenses:Home:Rent 1800.00 USD".to_owned(), checking.clone()];
            self.transaction(self.day(month, 3), None, "Landlord", "rent", &rent);
            let top_ups = [
                format!("Assets:Bank:EUR 400.00 EUR @ {} USD", decimal(eur, 4)),
                format!("Assets:Cash:JPY 40000 JPY @ {} USD", decimal(jpy, 6)),
                // USD per CNY, from CNY per USD
                format!("Assets:Bank:CNY 3000.00 CNY @ {} USD", decimal(100_000_000 / cny, 4)),
            ];
            for top_up in top_ups {
                self.transaction(self.day(month, 4), None, "Bank", "currency top-up", &[top_up, checking.clone()]);
            }
            let streaming = ["Expenses:Fun:Streaming 15.99 USD".to_owned(), "Liabilities:CreditCard".to_owned()];
            self.transaction(self.day(month, 7), None, "Netflix", "streaming", &streaming);
            let shares = [format!("Assets:Brokerage 2 VTI {{{} USD}}", decimal(vti, 2)), checking.clone()];
            self.transaction(self.day(month, 10), None, "Broker", "shares", &shares);
            let insurance = ["Expenses:Insurance:Health 250.00 USD".to_owned(), checking.clone()];
            self.transaction(self.day(month, 20), None, "Blue Shield", "health insurance", &insurance);
            let payment = 1500 + rng.below(2501) as i64;
            let card = [format!("Liabilities:CreditCard {payment}.00 USD"), checking.clone()];
            self.transaction(self.day(month, 25), None, "Bank", "card payment", &card);
        }
    }

    /// the first and the last day an account of `window` takes postings
    fn window(&self, window: Window) -> (NaiveDate, NaiveDate) {
        let (start, end) = (self.spec.first_day().succ_opt().expect("a date"), self.spec.last_day());
        let before = |date: NaiveDate| date.pred_opt().expect("a date");
        match window {
            Window::Always => (start, end),
            Window::Gym => (start, before(self.day(self.plan.gym_close, 28))),
            Window::Laptop => (start, before(self.day(self.plan.laptop_close, 15))),
            Window::Wedding => (self.day(self.plan.wedding_from, 2), before(self.day(self.plan.wedding_close, 20))),
        }
    }

    /// the spending transactions, on random days of the ledger after its first
    fn random_transactions(&mut self) {
        let mut rng = stream(self.spec.seed, 3);
        let count = self.spec.transactions.saturating_sub(self.routine_count());
        let days = (self.spec.last_day() - self.spec.first_day()).num_days() as usize;
        let windows = SPENDING.iter().map(|it| self.window(it.window)).collect::<Vec<_>>();
        for _ in 0..count {
            let date = self.spec.first_day() + Days::new(1 + rng.below(days) as u64);
            let time = rng
                .chance(30)
                .then(|| NaiveTime::from_num_seconds_from_midnight_opt(rng.below(86_400) as u32, 0).expect("a time"));
            let open = |index: &usize| windows[*index].0 <= date && date <= windows[*index].1;
            let candidates = (0..SPENDING.len()).filter(open).collect::<Vec<_>>();
            let spending = &SPENDING[pick_weighted(&mut rng, &candidates)];
            let cents = between(&mut rng, spending.range);
            let payee = *rng.pick(spending.payees);
            let (posting, funding) = self.posting(&mut rng, spending, date, cents);
            let mut postings = vec![posting];
            // a tenth of the USD ones buy something of another category too
            if funding.is_none() && rng.chance(10) {
                let usd = candidates.iter().copied().filter(|it| SPENDING[*it].mix == Mix::Usd).collect::<Vec<_>>();
                let other = &SPENDING[pick_weighted(&mut rng, &usd)];
                if other.account != spending.account {
                    postings.push(format!("{} {}", other.account, Commodity::Usd.amount(between(&mut rng, other.range))));
                }
            }
            let funding = funding.unwrap_or_else(|| {
                match rng.below(10) {
                    0..=5 => "Liabilities:CreditCard",
                    6..=8 => "Assets:Bank:Checking",
                    _ => "Assets:Cash",
                }
                .to_owned()
            });
            postings.push(funding);
            self.transaction(date, time, payee, spending.narration, &postings);
        }
    }

    /// The posting of a spending of about `cents` US cents, in a currency of the account's mix, and the account it
    /// is paid from when it is not a USD one.
    fn posting(&self, rng: &mut XorShift, spending: &Spending, date: NaiveDate, cents: i64) -> (String, Option<String>) {
        let account = spending.account;
        let in_currency = |commodity: Commodity| format!("{account} {}", commodity.amount(commodity.units_of_cents(cents)));
        let eur = || (in_currency(Commodity::Eur), Some("Assets:Bank:EUR".to_owned()));
        let gbp = || {
            let rate = self.prices_at(date).gbp;
            let posting = format!("{} @ {} USD", in_currency(Commodity::Gbp), decimal(rate, 4));
            (posting, Some("Liabilities:CreditCard".to_owned()))
        };
        let usd = || (in_currency(Commodity::Usd), None);
        let usd_card = || (in_currency(Commodity::Usd), Some("Liabilities:CreditCard".to_owned()));
        let gold = |rng: &mut XorShift| {
            let units = 1 + rng.below(2);
            let cost = self.prices_at(date).gold;
            let posting = format!("{account} {units} GOLD {{{} USD}}", decimal(cost, 2));
            (posting, Some("Assets:Bank:Checking".to_owned()))
        };
        let roll = rng.below(100);
        match spending.mix {
            Mix::Usd => match roll {
                0..=91 => usd(),
                92..=96 => eur(),
                _ => gbp(),
            },
            Mix::Europe => match roll {
                0..=74 => eur(),
                _ => usd_card(),
            },
            Mix::Souvenirs => match roll {
                0..=69 => eur(),
                _ => gold(rng),
            },
            Mix::Japan => match roll {
                0..=84 => (in_currency(Commodity::Jpy), Some("Assets:Cash:JPY".to_owned())),
                _ => usd_card(),
            },
            Mix::China => match roll {
                0..=69 => (in_currency(Commodity::Cny), Some("Assets:Bank:CNY".to_owned())),
                _ => (in_currency(Commodity::Usd), Some("Assets:Bank:Checking".to_owned())),
            },
            Mix::Gifts => match roll {
                0..=89 => usd(),
                _ => gold(rng),
            },
            Mix::Flights => match roll {
                0..=69 => usd_card(),
                70..=89 => eur(),
                _ => gbp(),
            },
        }
    }

    // ---- accounts and budgets -----------------------------------------------------------------------------------

    /// the `open` of `account`, with the `budget` metadata of `budgets` when the ledger has budgets
    fn open(&self, date: NaiveDate, account: &str, constraint: &str, budgets: &[&str]) -> String {
        let mut text = format!("{date} open {account}");
        if !constraint.is_empty() {
            write!(text, " {constraint}").expect("a string");
        }
        text.push('\n');
        for budget in budgets.iter().filter(|it| self.selected(it)) {
            writeln!(text, "  budget: \"{budget}\"").expect("a string");
        }
        text
    }

    /// the budgets of the spec whose first `open` links `account`
    fn first_budgets(&self, account: &str) -> Vec<&'static str> {
        BUDGETS[..self.spec.budgets]
            .iter()
            .filter(|budget| budget.accounts.contains(&account))
            .map(|budget| budget.name)
            .collect()
    }

    /// the accounts opened after the first day: the two closed at the end of a month and opened again on the first
    /// day of the next with other budgets, and the account of the budget defined later
    fn later_opens(&mut self) {
        for (account, budgets) in REOPENED {
            let month = match *account {
                "Expenses:Transport:Taxi" => self.plan.taxi_reopen,
                _ => self.plan.coffee_reopen,
            };
            let reopen = self.day(month, 1);
            let close = reopen.pred_opt().expect("a date");
            self.push(close, Class::Close, None, format!("{close} close {account}\n"));
            let text = self.open(reopen, account, "", budgets);
            self.push(reopen, Class::Open, None, text);
        }
        let wedding = self.day(self.plan.wedding_from, 1);
        let text = self.open(wedding, "Expenses:Goals:Wedding", "", &self.first_budgets("Expenses:Goals:Wedding"));
        self.push(wedding, Class::Open, None, text);
    }

    /// the `budget` directive of a budget
    fn definition(date: NaiveDate, budget: &BudgetDef) -> String {
        let mut text = format!("{date} budget {} {}\n", budget.name, budget.commodity.code());
        if let Some(alias) = budget.alias {
            writeln!(text, "  alias: \"{alias}\"").expect("a string");
        }
        writeln!(text, "  category: \"{}\"", budget.category).expect("a string");
        text
    }

    /// the day a budget is defined
    fn defined(&self, budget: &BudgetDef) -> NaiveDate {
        match budget.name {
            "Wedding" => self.day(self.plan.wedding_from, 1),
            _ => self.spec.first_day(),
        }
    }

    /// the day a budget closes, with its time, and the budget what is left of it goes to
    fn closed(&self, budget: &BudgetDef) -> Option<(NaiveDate, Option<NaiveTime>, &'static str)> {
        let (date, time, to) = match budget.name {
            "Gym" => (self.day(self.plan.gym_close, 28), NaiveTime::from_hms_opt(18, 0, 0), "Medical"),
            "New-Laptop" => (self.day(self.plan.laptop_close, 15), None, "Electronics"),
            "Wedding" => (self.day(self.plan.wedding_close, 20), None, "Gifts"),
            "Taxi" => (self.day(self.plan.taxi_reopen, 1).pred_opt().expect("a date"), None, "Transit"),
            _ => return None,
        };
        Some((date, time, to))
    }

    /// whether a budget of the spec takes `budget-add`s and transfers on `date`
    fn active(&self, budget: &BudgetDef, date: NaiveDate) -> bool {
        self.defined(budget) <= date && self.closed(budget).is_none_or(|(close, _, _)| date <= close)
    }

    /// the budgets defined after the first day, their `budget-add`s and `budget-transfer`s, their closes and the
    /// transfers of what is left after them
    fn budget_directives(&mut self) {
        let mut rng = stream(self.spec.seed, 4);
        let budgets = &BUDGETS[..self.spec.budgets];
        for budget in budgets {
            let defined = self.defined(budget);
            if defined > self.spec.first_day() {
                self.push(defined, Class::Budget, None, Self::definition(defined, budget));
            }
            if let Some((date, time, to)) = self.closed(budget) {
                let text = format!("{} budget-close {}\n", Self::stamp(date, time), budget.name);
                self.push(date, Class::BudgetClose, time, text);
                if self.selected(to) {
                    let after = date + Days::new(1);
                    let amount = Commodity::Usd.amount(100 * between(&mut rng, (20, 80)));
                    let text = format!("{after} budget-transfer {} {to} {amount}\n", budget.name);
                    self.push(after, Class::BudgetTransfer, None, text);
                }
            }
        }
        for month in 0..self.spec.months {
            let first = self.day(month, 1);
            let active = budgets.iter().filter(|it| self.active(it, first)).collect::<Vec<_>>();
            for budget in active {
                // a whole amount around the monthly one (whole dollars, euros or yuan, hundreds of yen), sometimes
                // in USD for a budget in another commodity
                let units = budget.monthly * (90 + rng.below(21) as i64) / 100 / 100 * 100;
                let whole = |cents: i64| cents / 100 * 100;
                let amount = match (budget.name, month % 12) {
                    ("Travel-Europe", 3 | 7 | 11) => Commodity::Usd.amount(whole(units * 100 / 92)),
                    ("Japan", 5 | 11) => Commodity::Usd.amount(whole(units * 100 / 145)),
                    _ => budget.commodity.amount(units),
                };
                let text = format!("{first} budget-add {} {amount}\n", budget.name);
                self.push(first, Class::BudgetAdd, None, text);
            }
            for _ in 0..2 + rng.below(3) {
                let date = self.day(month, 2 + rng.below(26) as u32);
                let usd = budgets
                    .iter()
                    .filter(|it| it.commodity == Commodity::Usd && self.active(it, date))
                    .collect::<Vec<_>>();
                if usd.len() < 2 {
                    continue;
                }
                let from = *rng.pick(&usd);
                let europe = budgets.iter().find(|it| it.name == "Travel-Europe" && self.active(it, date));
                let to = match europe {
                    Some(europe) if rng.chance(10) => europe,
                    _ => *rng.pick(&usd),
                };
                if from.name == to.name {
                    continue;
                }
                let amount = Commodity::Usd.amount(100 * between(&mut rng, (10, 100)));
                let text = format!("{date} budget-transfer {} {} {amount}\n", from.name, to.name);
                self.push(date, Class::BudgetTransfer, None, text);
            }
        }
    }

    // ---- files --------------------------------------------------------------------------------------------------

    fn files(mut self) -> GeneratedLedger {
        let first = self.spec.first_day();
        let last_year = self.spec.last_day().year();
        let mut main = String::new();
        writeln!(
            main,
            "option \"title\" \"Budget baseline ledger (seed {}, {} transactions, {} months)\"",
            self.spec.seed, self.spec.transactions, self.spec.months
        )
        .expect("a string");
        main.push_str("option \"operating_currency\" \"USD\"\n");
        writeln!(main, "option \"timezone\" \"{TIMEZONE}\"").expect("a string");
        main.push('\n');
        for year in first.year()..=last_year {
            writeln!(main, "include \"{year}.zhang\"").expect("a string");
        }
        main.push('\n');
        for (code, precision) in [("USD", 2), ("EUR", 2), ("GBP", 2), ("JPY", 0), ("CNY", 2), ("GOLD", 2), ("VTI", 2)] {
            writeln!(main, "1970-01-01 commodity {code}\n  precision: {precision}").expect("a string");
        }
        main.push('\n');
        for (account, constraint) in OTHER_ACCOUNTS {
            main.push_str(&self.open(first, account, constraint, &self.first_budgets(account)));
        }
        for spending in SPENDING.iter().filter(|it| it.window != Window::Wedding) {
            main.push_str(&self.open(first, spending.account, "", &self.first_budgets(spending.account)));
        }
        if self.spec.with_budgets {
            main.push('\n');
            for budget in BUDGETS[..self.spec.budgets].iter().filter(|it| self.defined(it) == first) {
                main.push_str(&Self::definition(first, budget));
            }
        }

        // a stable sort: the events of a day keep the order they were made in within their class and time
        self.events.sort_by_key(|event| (event.date, event.class, event.seconds));
        let mut years = (first.year()..=last_year).map(|year| (year, String::new())).collect::<Vec<_>>();
        for event in &self.events {
            let year = (event.date.year() - first.year()) as usize;
            years[year].1.push_str(&event.text);
        }
        let mut files = years.into_iter().map(|(year, text)| (format!("{year}.zhang"), text)).collect::<Vec<_>>();
        files.push((MAIN.to_owned(), main));
        files.sort_by(|a, b| a.0.cmp(&b.0));
        GeneratedLedger { files }
    }
}

/// a number in `range`, both ends included
fn between(rng: &mut XorShift, range: (i64, i64)) -> i64 {
    range.0 + rng.below((range.1 - range.0 + 1) as usize) as i64
}

/// one of the spending accounts `candidates` (indices of [`SPENDING`]), by weight
fn pick_weighted(rng: &mut XorShift, candidates: &[usize]) -> usize {
    let total = candidates.iter().map(|it| SPENDING[*it].weight).sum::<u64>();
    let mut roll = rng.below(total as usize) as u64;
    for candidate in candidates {
        let weight = SPENDING[*candidate].weight;
        if roll < weight {
            return *candidate;
        }
        roll -= weight;
    }
    unreachable!("the roll is below the total weight")
}
