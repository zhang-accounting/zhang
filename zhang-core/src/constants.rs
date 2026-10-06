use zhang_ast::Rounding;

pub const KEY_OPERATING_CURRENCY: &str = "operating_currency";
pub const KEY_DEFAULT_ROUNDING: &str = "default_rounding";
pub const KEY_DEFAULT_COMMODITY_PRECISION: &str = "default_commodity_precision";
pub const KEY_DEFAULT_BOOKING_METHOD: &str = "default_booking_method";
pub const KEY_FEATURES_PLUGIN: &str = "features.plugin";
/// alias of [KEY_FEATURES_PLUGIN]
pub const KEY_FEATURES_PLUGINS: &str = "features.plugins";

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

/// the payee a balance assertion is listed under in the journals; its narration is the account
pub const BALANCE_CHECK_PAYEE: &str = "Balance Check";

pub const COMMODITY_GROUP: &str = "group";

/// zhang's cache folder, relative to the working directory: serving from the ledger root puts it under the root. It
/// holds the documents a server read from a remote source
pub const CACHE_DIR: &str = ".cache";

/// `{{ext}}` is the main file's extension, so new directives are written in the ledger's own format
/// (`.bean` files for a `main.bean` ledger, `.zhang` files for a `main.zhang` one).
pub const DEFAULT_DIRECTIVE_OUTPUT_PATH: &str = r#"data/{{year}}/{{month_str}}.{{ext}}"#;
