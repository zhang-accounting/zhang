#[cfg(feature = "openapi")]
use gotcha_core::Schematic;
use serde::Serialize;
use strum::{Display, EnumString};

#[derive(Debug, Display, EnumString, Clone, PartialEq, Eq, Serialize)]
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
    /// a notice: a `balance` of a beancount file whose `time` metadata is later than transactions of its account on
    /// its day. Zhang checks it at the start of its date, as beancount does, before those transactions
    BalanceTimeIgnored,
    /// a notice: a `document` of a beancount file whose path is not found relative to that file, as beancount reads
    /// it, but is found relative to the ledger's root, as earlier versions of zhang wrote it. Zhang uses the file there
    DocumentPathRelativeToRoot,
    /// a `document` of a beancount ledger whose file does not exist, as beancount reports it
    DocumentNotFound,
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
    /// a cost spec with beancount's merge-cost marker `*` (`{*}`): cost merging is not supported, and the posting
    /// books as if its spec had no marker, as in beancount
    CostMergingNotSupported,

    /// a WASM plugin reported a problem through the `zhang_emit_error` host function
    PluginError,
}
