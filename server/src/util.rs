use bigdecimal::BigDecimal;
use zhang_core::domains::schemas::{AccountBalanceDomain, AccountDailyBalanceDomain};

pub trait AmountLike {
    fn number(&self) -> &BigDecimal;

    fn commodity(&self) -> &String;
}

impl AmountLike for AccountDailyBalanceDomain {
    fn number(&self) -> &BigDecimal {
        &self.balance_number.0
    }

    fn commodity(&self) -> &String {
        &self.balance_commodity
    }
}

impl AmountLike for AccountBalanceDomain {
    fn number(&self) -> &BigDecimal {
        &self.balance_number.0
    }

    fn commodity(&self) -> &String {
        &self.balance_commodity
    }
}
