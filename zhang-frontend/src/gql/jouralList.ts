import { gql } from "@apollo/client";
import { AccountItem } from "./accountList";


export interface JouralListQuery {
    journals: JournalItem[]
}

export type JournalItem = TransactionDto | BalanceCheckDto | BalancePadDto;

export interface TransactionDto {
    date: string,
    type: "TransactionDto"
    payee: string
    narration?: string
    postings: Posting[]
}

export interface Posting {
    account: AccountItem,
    unit: Amount
}

export interface Amount {
    number: string,
    currency: string
}

export interface BalanceCheckDto {
    date: string,
    type: "BalanceCheckDto"
    account: AccountItem,

    balanceAmount: Amount,
    currentAmount: Amount
    isBalanced: boolean
    distance: Amount
}

export interface BalancePadDto {
    date: string,
    type: "BalancePadDto"
}

export const JOURNAL_LIST = gql`
query JOURNAL_LIST {
  journals {
    date
    type: __typename
    ... on TransactionDto {
      payee
      narration
      postings {
        account {
          name
        }
        unit {
          number
          currency
        }
      }
    }
    ... on BalanceCheckDto {
      account {
        name
      }
      balanceAmount {
        number
        currency
      }
      currentAmount {
        number
        currency
      }
      isBalanced
      distance {
        number
        currency
      }
    }
  }
}

`