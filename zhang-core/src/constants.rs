use zhang_ast::Rounding;

pub const KEY_OPERATING_CURRENCY: &str = "operating_currency";
pub const KEY_DEFAULT_ROUNDING: &str = "default_rounding";
pub const KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION: &str = "default_balance_tolerance_precision";
pub const KEY_DEFAULT_COMMODITY_PRECISION: &str = "default_commodity_precision";
pub const KEY_TIMEZONE: &str = "timezone";

pub const KEY_FEATURES_PLUGIN: &str = "features.plugin";

pub const DEFAULT_COMMODITY_PRECISION: i32 = 2;
pub const DEFAULT_OPERATING_CURRENCY: &str = "CNY";
pub const DEFAULT_ROUNDING: Rounding = Rounding::RoundDown;
pub const DEFAULT_BALANCE_TOLERANCE_PRECISION: i32 = 2;
pub const DEFAULT_TIMEZONE: &str = "Asia/Hong_Kong";

pub const DEFAULT_ROUNDING_PLAIN: &str = "RoundDown";
pub const DEFAULT_COMMODITY_PRECISION_PLAIN: &str = "2";
pub const DEFAULT_BALANCE_TOLERANCE_PRECISION_PLAIN: &str = "2";

pub const DEFAULT_BOOKING_METHOD: &str = "FIFO";

pub const TRUE: &str = "true";

pub const TXN_ID: &str = "txn_id";

pub const COMMODITY_GROUP: &str = "group";
