import { gql } from "@apollo/client";
import { Statistic } from "../models";


export interface StatisticResponse {
    statistic: Statistic
}


export const STATISTIC = gql`
query STATISTIC($from: Int, $to: Int, $gap: Int) {
    statistic(from: $from, to: $to) {
      start
      end
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
`