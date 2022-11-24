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
