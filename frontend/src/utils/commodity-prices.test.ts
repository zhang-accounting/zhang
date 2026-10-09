// Tests of the price history chart's data, run with Node's built-in runner (Node >= 23.6 strips the types): pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { priceHistoryByQuote } from './commodity-prices.ts';

// local times without an offset: the chart buckets records by local calendar day, as the page lists them
const at = (datetime: string, number: string, commodity: string) => ({ datetime, amount: { number, commodity } });

test('a commodity quoted in two currencies gets one history per quote, each with only its own prices', () => {
  // AAPL at 150 USD and 1,080 CNY: on one axis the USD line would be flat along the bottom
  const prices = [at('2026-10-02T12:00:00', '150', 'USD'), at('2026-10-03T12:00:00', '155.5', 'USD'), at('2026-10-04T12:00:00', '1080', 'CNY')];
  assert.deepEqual(priceHistoryByQuote(prices), [
    { quote: 'CNY', points: [{ date: '2026-10-04', price: 1080 }] },
    {
      quote: 'USD',
      points: [
        { date: '2026-10-02', price: 150 },
        { date: '2026-10-03', price: 155.5 },
      ],
    },
  ]);
});

test('the points of a quote are one per day in ascending order, the last record of a day winning', () => {
  const prices = [
    at('2026-10-03T12:00:00', '3', 'USD'),
    at('2026-10-01T09:00:00', '1', 'USD'),
    at('2026-10-01T18:00:00', '1.5', 'USD'),
    at('2026-10-02T12:00:00', '2', 'USD'),
  ];
  assert.deepEqual(priceHistoryByQuote(prices), [
    {
      quote: 'USD',
      points: [
        { date: '2026-10-01', price: 1.5 },
        { date: '2026-10-02', price: 2 },
        { date: '2026-10-03', price: 3 },
      ],
    },
  ]);
  assert.deepEqual(priceHistoryByQuote([]), []);
});
