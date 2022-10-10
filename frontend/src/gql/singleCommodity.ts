import { gql } from '@apollo/client';
import { Currency } from './accountList';

export interface SingleCommodityQuery {
  currency: Currency;
}

export const SINGLE_COMMODITIY = gql`
  query ($name: String) {
    currency(name: $name) {
      name
      precision
      lots {
        lotCurrency
        lotPrice
        number
      }
      priceHistories {
        date
        amount {
          number
          currency
        }
      }
    }
  }
`;
