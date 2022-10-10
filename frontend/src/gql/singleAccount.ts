import { gql } from '@apollo/client';
import { AccountItem } from './accountList';

export interface SingleAccountJournalQuery {
  account: AccountItem;
}

export const SINGLE_ACCONT_JOURNAL = gql`
  query SINGLE_ACCONT_JOURNAL($name: String) {
    account(name: $name) {
      name
      status
      currencies {
        name
      }
      snapshot {
        detail {
          number
          currency
        }
      }
      documents {
        filename
        __typename
      }
      latestBalanceTimes {
        commodity
        date
      }
      journals {
        date
        type: __typename
        ... on TransactionDto {
          timestamp
          payee
          narration
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
