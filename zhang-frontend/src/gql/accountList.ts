import { gql } from "@apollo/client";
import { Snapshot } from "src/models";
import { JournalItem } from "./jouralList";


export interface AccountListQuery {
  accounts: AccountItem[]
}

export interface AccountItem {
  name: string,
  status: "OPEN" | "CLOSE",
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
  filename: string,
  __typename: "AccountDocumentDto"
  account: AccountItem
}

export interface TransactionDocumentDto {
  filename: string,
  __typename: "TransactionDocumentDto"
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