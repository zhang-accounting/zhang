import { gql } from '@apollo/client';
import { Statistic } from '../models';

export interface StatisticResponse {
  statistic: Statistic;
}

export const STATISTIC = gql`
  query STATISTIC($from: Int, $to: Int, $gap: Int) {
    statistic(from: $from, to: $to) {
      start
      end
      journals(transactionOnly: true) {
        type: __typename
        
        ... on TransactionDto {
          date
          timestamp
          payee
          narration
          postings {
            unit {
              currency
              number
            }
            account {
              name
              accountType
              sign
            }
          }
        }
      }
      income: distance(accounts: ["Income"]) {
        summary {
          number
          currency
        }
      }
      expense: distance(accounts: ["Expenses"]) {
        summary {
          number
          currency
        }
      }
      incomeExpense: distance(accounts: ["Income", "Expenses"]) {
        summary {
          number
          currency
        }
      }
      total: categorySnapshot(categories: ["Assets", "Liabilities"]) {
        summary {
          number
          currency
        }
      }
      frames(gap: $gap) {
        start
        end
        income: distance(accounts: ["Income"]) {
          summary {
            number
            currency
          }
        }
        expense: distance(accounts: ["Expenses"]) {
          summary {
            number
            currency
          }
        }

        total: categorySnapshot(categories: ["Assets", "Liabilities"]) {
          summary {
            number
            currency
          }
        }
      }
    }
  }
`;
