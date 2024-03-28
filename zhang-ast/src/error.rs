use serde::Serialize;
use strum::Display;

#[derive(Debug, Display, Clone, PartialEq, Eq, Serialize)]
pub enum ErrorKind {
    UnbalancedTransaction,
    TransactionCannotInferTradeAmount,
    TransactionHasMultipleImplicitPosting,
    TransactionExplicitPostingHaveMultipleCommodity,
    InvalidFlag,

    AccountBalanceCheckError,
    AccountDoesNotExist,
    AccountClosed,
    TransactionDoesNotBalance,
    CommodityDoesNotDefine,
    CloseNonZeroAccount,

    BudgetDoesNotExist,

    MultipleOperatingCurrencyDetect,
}
