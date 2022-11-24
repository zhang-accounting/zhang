use bigdecimal::BigDecimal;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "type")]
pub enum AccountBalanceRequest {
    Check {
        amount: BigDecimal,
        commodity: String,
    },
    Pad {
        amount: BigDecimal,
        commodity: String,
        pad_account: String,
    },
}
