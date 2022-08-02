import { gql } from "@apollo/client";
import { Snapshot } from "../models";
import { Amount, JournalItem, TransactionDto } from "./jouralList";


export interface AccountListQuery {
  accounts: AccountItem[]
}

export interface AccountItem {
  name: string,
  status: "OPEN" | "CLOSE",
  accountType: "Liabilities" | "Expenses" | "Income" | "Assets" | "Equity",
  sign: 1 | -1,
  snapshot: Snapshot,
  documents: Document[],
  journals: JournalItem[]
  currencies: Currency[]
}

export interface Currency {
  name: string,
  precision: number,
  balance: string,
  isOperatingCurrency: boolean,
  latestPrice: Price,
  lots: Lot[],
  priceHistories: Price[]
}

export interface Lot {
  lotCurrency: string,
  lotPrice: string,
  number: string
}

export interface Price {
  date: number,
  amount: Amount
}

export type Document = AccountDocumentDto | TransactionDocumentDto;

export interface AccountDocumentDto {
  date: number,
  filename: string,
  __typename: "AccountDocumentDto"
  account?: AccountItem
}

export interface TransactionDocumentDto {
  date: number,
  filename: string,
  __typename: "TransactionDocumentDto",
  transaction: TransactionDto
}

export const ACCOUNT_LIST = gql`
query {
    accounts {
      name
      status
      snapshot {
        summary{
          number
          currency
        }
      }
      currencies {
        name
      }
    }
  }
     
`