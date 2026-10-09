// The price history of a commodity's page, as its chart draws it. Type-only imports keep this module runnable by
// `node --test` (commodity-prices.test.ts).
import BigNumber from 'bignumber.js';
import { format } from 'date-fns';

/** A price record of `GET /api/commodities/{name}`: the commodity's price in the quote commodity `amount.commodity`. */
export interface PriceRecord {
  /** RFC 3339 */
  datetime: string;
  amount: { number: string; commodity: string };
}

export interface PricePoint {
  /** the local calendar day, `YYYY-MM-DD` */
  date: string;
  price: number;
}

/** The history in one quote commodity: a chart of its own, on its own scale. */
export interface QuotePriceHistory {
  quote: string;
  /** one point per day in ascending order; the last record of a day is the day's price */
  points: PricePoint[];
}

/**
 * The history per quote commodity, quotes in alphabetical order. A commodity quoted in two currencies (AAPL at 150 USD
 * and 1,080 CNY) is drawn as one chart per quote: on a shared axis the smaller quote would flatten into a line along
 * the bottom.
 */
export function priceHistoryByQuote(prices: PriceRecord[]): QuotePriceHistory[] {
  const byQuote = new Map<string, Map<string, number>>();
  [...prices]
    .sort((a, b) => new Date(a.datetime).getTime() - new Date(b.datetime).getTime())
    .forEach((price) => {
      const byDate = byQuote.get(price.amount.commodity) ?? new Map<string, number>();
      byDate.set(format(new Date(price.datetime), 'yyyy-MM-dd'), new BigNumber(price.amount.number).toNumber());
      byQuote.set(price.amount.commodity, byDate);
    });
  return Array.from(byQuote, ([quote, byDate]) => ({ quote, points: Array.from(byDate, ([date, price]) => ({ date, price })) })).sort((a, b) =>
    a.quote.localeCompare(b.quote),
  );
}
