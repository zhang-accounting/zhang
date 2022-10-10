import { gql } from '@apollo/client';
import { AccountItem } from './accountList';

export interface Paginable<T> {
  data: T[];
}

export interface JouralListQuery {
  journals: Paginable<JournalItem>;
}

export type JournalItem = TransactionDto | BalanceCheckDto | BalancePadDto;

export interface TransactionDto {
  date: string;
  timestamp: number;
  type: 'TransactionDto';
  payee: string;
  narration?: string;
  postings: Posting[];
  tags: string[];
  links: string[];
  metas: Meta[];
  isBalanced: boolean;
}

export interface Posting {
  account: AccountItem;
  unit: Amount;
  inferredUnit: Amount;
}

export interface Amount {
  number: string;
  currency: string;
}

export interface BalanceCheckDto {
  date: string;
  type: 'BalanceCheckDto';
  account: AccountItem;

  balanceAmount: Amount;
  currentAmount: Amount;
  isBalanced: boolean;
  distance: Amount;
}

export interface BalancePadDto {
  date: string;
  type: 'BalancePadDto';
}

export interface Meta {
  key: string;
  value: string;
}

export const JOURNAL_LIST = gql`
  query JOURNAL_LIST($page: Int) {
    journals(page: $page, size: 100) {
      pageInfo {
        page
        total
        size
      }
      data {
        date
        type: __typename
        ... on TransactionDto {
          timestamp
          payee
          narration
          tags
          links
          isBalanced
          postings {
            account {
              name
              accountType
            }
            unit {
              number
              currency
            }
            inferredUnit {
              number
              currency
            }
          }
          metas {
            key
            value
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
  }
`;
