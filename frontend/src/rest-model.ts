export enum LoadingState {
  NotReady = 'NotReady',
  Loading = 'Loading',
  Success = 'Success',
  Refreshing = 'Refreshing',
}

export interface Pageable<T> {
  total_count: number;
  total_page: number;
  page_size: number;
  current_page: number;
  records: T[];
}

export enum AccountStatus {
  Open = 'Open',
  Close = 'Close',
}

export enum AccountType {
  Expenses = 'Expenses',
  Assets = 'Assets',
  Liabilities = 'Liabilities',
  Equity = 'Equity',
  Income = 'Income',
}

export interface Account {
  name: string;
  status: AccountStatus;
  alias?: String;
  amount: CalculatedAmountResponse;
}

export interface AccountInfo {
  name: string;
  status: AccountStatus;
  alias?: String;
  amount: CalculatedAmountResponse;
}
export interface AccountBalanceHistory {
  [commodity: string]: AccountBalanceHistoryItem[];
}

export interface AccountBalanceHistoryItem {
  date: string;
  balance: AmountCommodityResponse;
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
  payee: string[];
  account_name: string[];
}

export interface AccountJournalItem {
  datetime: string;
  trx_id: string;
  account: string;
  payee: string;
  narration?: string;
  inferred_unit_number: string;
  inferred_unit_commodity: string;
  account_after_number: number;
  account_after_commodity: string;
}

export interface CommodityListItem {
  name: string;
  precision: number;
  prefix: string;
  suffix: string;
  rounding: string;
  total_amount: string;
  group?: string;
  latest_price_date: string;
  latest_price_amount: string;
  latest_price_commodity: string;
}

export interface CommodityDetail {
  info: CommodityListItem;
  lots: CommodityLot[];

  prices: CommodityPrice[];
}

export interface CommodityLot {
  datetime?: string;
  amount: string;
  price_amount?: string;
  price_commodity?: string;
  account: string;
}

export interface CommodityPrice {
  datetime: string;
  amount: string;
  target_commodity: string;
}

export type JournalItem = JournalTransactionItem | JournalBlancePadItem | JournalBalanceCheckItem;

export interface JournalBlancePadItem {
  type: 'BalancePad';
  id: string;
  datetime: string;
  payee: string;
  narration?: string;
  tags: any[];
  links: any[];
  flag: string;
  postings: Posting[];
  metas: Meta[];
}

export interface JournalBalanceCheckItem {
  type: 'BalanceCheck';
  id: string;
  datetime: string;
  payee: string;
  narration?: string;
  tags: any[];
  links: any[];
  flag: string;
  postings: Posting[];
  metas: Meta[];
}

export interface Meta {
  key: string;
  value: string;
}

export interface JournalTransactionItem {
  type: 'Transaction';
  id: string;
  datetime: string;
  payee: string;
  narration?: string;
  tags: any[];
  links: any[];
  flag: string;
  is_balanced: boolean;
  postings: Posting[];
  metas: Meta[];
}

export interface Posting {
  account: string;
  unit_number?: string;
  unit_commodity?: string;
  cost_number: string;
  cost_commodity: string;
  price_number: string;
  price_commodity: string;
  inferred_unit_number: string;
  inferred_unit_commodity: string;
  account_before_number: string;
  account_before_commodity: string;
  account_after_number: string;
  account_after_commodity: string;
}

export interface StatisticResponse {
  balance: CalculatedAmountResponse;
  liability: CalculatedAmountResponse;
  income: CalculatedAmountResponse;
  expense: CalculatedAmountResponse;
  transaction_number: number;
}

export interface StatisticGraphResponse {
  // todo: to be deleted
  balances: { [date: string]: CalculatedAmountResponse };
  changes: { [date: string]: { [accountType: string]: CalculatedAmountResponse } };
}

export interface CalculatedAmountResponse {
  calculated: AmountResponse;
  detail: { [commodity: string]: string };
}

export interface AmountResponse {
  number: string;
  currency: string;
}

export interface AmountCommodityResponse {
  number: string;
  commodity: string;
}

export interface CurrentStatisticResponse {
  balance: CalculatedAmountResponse;
  liability: CalculatedAmountResponse;
  income: AmountResponse;
  expense: AmountResponse;
}

export interface StatisticTypeResponse {
  detail: { account: string; amount: CalculatedAmountResponse }[];
  // income_rank: { account: string; percent: string }[];
  top_transactions: AccountJournalItem[];
}

export interface SpanInfo {
  start: number;
  end: number;
  content: string;
  filename: string;
}

export interface Option {
  key: string;
  value: string;
}

export interface BudgetListItem {
  name: string;
  alias?: string;
  category?: string;
  closed: boolean;
  assigned_amount: AmountResponse;
  activity_amount: AmountResponse;
  available_amount: AmountResponse;
}

export interface BudgetInfoResponse {
  name: string;
  alias?: string;
  category?: string;
  closed: boolean;

  related_accounts: string[];
  assigned_amount: AmountResponse;
  activity_amount: AmountResponse;
  available_amount: AmountResponse;
}

export type BudgetIntervalEventResponse = BudgetIntervalEventBudgetResponse | BudgetIntervalEventPostingResponse;

export interface BudgetIntervalEventBudgetResponse {
  datetime: string;
  timestamp: number;
  amount: AmountResponse;
  event_type: string;
}

export interface BudgetIntervalEventPostingResponse {
  datetime: string;
  timestamp: number;
  account: string;
  trx_id: string;
  payee: string;
  narration?: string;
  inferred_unit_number: string;
  inferred_unit_commodity: string;
}

export interface PluginResponse {
  name: string;
  version: string;
  plugin_type: string[];
}
