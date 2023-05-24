use zhang_ast::Rounding;

pub const KEY_OPERATING_CURRENCY: &str = "operating_currency";
pub const KEY_DEFAULT_ROUNDING: &str = "default_rounding";
pub const KEY_DEFAULT_BALANCE_TOLERANCE_PRECISION: &str = "default_balance_tolerance_precision";

pub const DEFAULT_COMMODITY_PRECISION: i32 = 2;
pub const DEFAULT_OPERATING_CURRENCY: &str = "CNY";
pub const DEFAULT_ROUNDING: Rounding = Rounding::RoundDown;
pub const DEFAULT_BALANCE_TOLERANCE_PRECISION: i32 = 2;
