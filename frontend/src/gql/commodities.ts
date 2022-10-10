import { gql } from '@apollo/client';
import { Currency } from './accountList';

export interface CommoditiesQuery {
  currencies: Currency[];
}

export const CURRENCIES = gql`
  query {
    currencies {
      name
      precision
      balance
      isOperatingCurrency
      latestPrice {
        date
        amount {
          number
          currency
        }
      }
    }
  }
`;
