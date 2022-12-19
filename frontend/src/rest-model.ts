export interface Pageable<T> {
    total_count: number,
    total_page: number,
    page_size: number,
    current_page: number,
    records: T[]
}

export enum AccountStatus {
    Open = 'Open',
    Close = 'Close',
}


export enum AccountType {
    Expenses = "Expenses",
    Assets = "Assets",
    Liabilities = "Liabilities",
    Equity = 'Equity',
    Income = "Income"

}

export interface Account {
    name: string;
    status: AccountStatus;
    commodities: { [commodity_name: string]: string };
}

export interface Document {
    datetime: string;
    filename: string;
    path: string;
    extension?: string;
    account?: string;
    trx_id: string;
}

export interface InfoForNewTransaction {
    payee: string[],
    account_name: string[]
}

export interface AccountJournalItem {
    datetime: string,
    trx_id: string,
    account: string,
    payee: string,
    narration?: string,
    inferred_unit_number: string,
    inferred_unit_commodity: string,
    account_after_number: number,
    account_after_commodity: string,
}

export interface CommodityListItem {
    name: string,
    precision: number,
    prefix: string,
    suffix: string,
    rounding: string,
    total_amount: string,
    latest_price_date: string,
    latest_price_amount: string,
    latest_price_commodity: string,
}

export interface CommodityDetail {
    info: CommodityListItem,
    lots: CommodityLot[],

    prices: CommodityPrice[],
}

export interface CommodityLot {
    datetime?: string,
    amount: string,
    price_amount?: string,
    price_commodity?: string,
    account: string,
}

export interface CommodityPrice {
    datetime: string,
    amount: string,
    target_commodity: string,
}

export type JournalItem = JournalTransactionItem | JournalBlancePadItem

export interface JournalBlancePadItem {
    type: "BalancePad"
    id: string,
    datetime: string
    payee: string
    narration?: string
    tags: any[]
    links: any[]
    flag: string
    is_balanced: boolean
    postings: Posting[],
    metas: Meta[],
}

export interface Meta {
    key: string,
    value: string
}

export interface JournalTransactionItem {
    type: "Transaction"
    id: string,
    datetime: string
    payee: string
    narration?: string
    tags: any[]
    links: any[]
    flag: string
    is_balanced: boolean
    postings: Posting[]
    metas: Meta[],
}

export interface Posting {
    account: string
    unit_number?: string
    unit_commodity?: string
    cost_number: string
    cost_commodity: string
    price_number: string
    price_commodity: string
    inferred_unit_number: string
    inferred_unit_commodity: string
    account_before_number: string
    account_before_commodity: string
    account_after_number: string
    account_after_commodity: string
}

export interface StatisticResponse {
    changes: { [date: string]: { [type: string]: AmountResponse } }
    details: { [date: string]: { [account: string]: AmountResponse } }
}

export interface AmountResponse {
    number: string,
    commodity: string
}

export interface CurrentStatisticResponse {
    balance: AmountResponse,
    liability: AmountResponse,
    income: AmountResponse,
    expense: AmountResponse
}


export interface ReportResponse {
    balance: AmountResponse,
    liability: AmountResponse,
    income: AmountResponse,
    expense: AmountResponse,
    transaction_number: number,

    income_rank: { account: string, percent: string }[]
    income_top_transactions: AccountJournalItem[]
    expense_rank: { account: string, percent: string }[]
    expense_top_transactions: AccountJournalItem[]
}

export enum LedgerErrorType {
    AccountBalanceCheckError = 'AccountBalanceCheckError',
    AccountDoesNotExist = 'AccountDoesNotExist',
    AccountClosed = 'AccountClosed',
    TransactionDoesNotBalance = "TransactionDoesNotBalance",
    CommodityDoesNotDefine = 'CommodityDoesNotDefine',
    TransactionHasMultipleImplicitPosting = 'TransactionHasMultipleImplicitPosting'
}

export interface SpanInfo {

    start: number,
    end: number,
    content: string,
    filename: string

}

export interface LedgerError {
    span: SpanInfo,
    error: AccountBalanceCheckError
        | AccountDoesNotExist
        | AccountClosed
        | TransactionDoesNotBalance
        | CommodityDoesNotDefine
        | TransactionHasMultipleImplicitPosting
}


export interface AccountBalanceCheckError extends SpanInfo {
    type: LedgerErrorType.AccountBalanceCheckError,
    account_name: string,
    target: AmountResponse,
    current: AmountResponse,
    distance: AmountResponse,

}

export interface AccountDoesNotExist extends SpanInfo {
    type: LedgerErrorType.AccountDoesNotExist,
    account_name: string,
}

export interface AccountClosed extends SpanInfo {
    type: LedgerErrorType.AccountClosed,
    account_name: string,
}

export interface TransactionDoesNotBalance extends SpanInfo {
    type: LedgerErrorType.TransactionDoesNotBalance,
}


export interface CommodityDoesNotDefine extends SpanInfo {
    type: LedgerErrorType.CommodityDoesNotDefine,
    commodity_name: string,
}

export interface TransactionHasMultipleImplicitPosting extends SpanInfo {
    type: LedgerErrorType.TransactionHasMultipleImplicitPosting,
}
