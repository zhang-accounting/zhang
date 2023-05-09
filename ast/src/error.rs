#[derive(Debug)]
pub enum ErrorKind {
    TransactionHasMultipleImplicitPosting,
    TransactionCannotInferTradeAmount,
}
