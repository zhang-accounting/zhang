#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use serde::Serialize;
use strum::Display;

#[derive(Debug, Display, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(Schematic))]
pub enum ErrorKind {
    UnbalancedTransaction,
    TransactionCannotInferTradeAmount,
    TransactionHasMultipleImplicitPosting,
    TransactionExplicitPostingHaveMultipleCommodity,

    AccountBalanceCheckError,
    /// a `pad` that pads nothing: no later balance assertion of its account needs it
    UnusedPad,
    /// a pad of a commodity its account holds at cost: the padding is booked without a cost
    PadWithCost,
    AccountDoesNotExist,
    AccountClosed,

    CommodityDoesNotDefine,
    NoEnoughCommodityLot,
    CloseNonZeroAccount,

    BudgetDoesNotExist,
    DefineDuplicatedBudget,

    MultipleOperatingCurrencyDetect,

    ParseInvalidMeta,

    UnsupportedBookingMethod,
    AmbiguousLotMatch,

    /// a WASM plugin reported a problem through the `zhang_emit_error` host function
    PluginError,
}
