export enum AccountStatus {
  Open = 'Open',
  Close = 'Close',
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
  trx_id: String,
  payee: String,
  narration?: string,
  inferred_unit_number: number,
  inferred_unit_commodity: String,
  account_after_number: number,
  account_after_commodity: String,
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
  datetime?: String,
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
  postings: Posting[]
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
}

export interface Posting {
  account: string
  unit_number?: string
  unit_commodity?: string
  cost_number: string
  cost_commodity: string
  price_number: string
  price_commodity: string
  inferred_unit_number:string
  inferred_unit_commodity: string
  account_before_number:string
  account_before_commodity: string
  account_after_number:string
  account_after_commodity: string
}
