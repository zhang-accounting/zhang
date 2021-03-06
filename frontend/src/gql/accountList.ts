import { gql } from "@apollo/client";
import { Snapshot } from "../models";
import { JournalItem, TransactionDto } from "./jouralList";


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
  name: string
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