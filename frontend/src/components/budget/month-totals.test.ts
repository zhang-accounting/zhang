import assert from 'node:assert/strict';
import { test } from 'node:test';
import { monthTotals, primaryFigures } from './month-totals.ts';

const cny = (number: string) => ({ number, commodity: 'CNY' });
const usd = (number: string) => ({ number, commodity: 'USD' });

function budget(assigned: { number: string; commodity: string }, activity: string, available: string, closed = false) {
  const amount = (number: string) => ({ number, commodity: assigned.commodity });
  return { closed, assigned_amount: assigned, activity_amount: amount(activity), available_amount: amount(available) };
}

const text = (totals: { commodity: string; number: { toString(): string } }[]) => totals.map((it) => `${it.number.toString()} ${it.commodity}`);

test('a budget closed in or before the month does not count in its totals', () => {
  // April: Food is open, Trip was closed in March and still has 50 CNY left
  const totals = monthTotals([budget(cny('300'), '120', '180'), budget(cny('50'), '0', '50', true)]);
  assert.deepEqual(text(totals.assigned), ['300 CNY']);
  assert.deepEqual(text(totals.activity), ['120 CNY']);
  assert.deepEqual(text(totals.available), ['180 CNY']);
});

test('the totals keep every commodity', () => {
  const totals = monthTotals([budget(cny('300'), '120', '180'), budget(usd('40'), '50', '-10')]);
  assert.deepEqual(text(totals.available), ['180 CNY', '-10 USD']);
  assert.deepEqual(text([primaryFigures(totals).assigned!, primaryFigures(totals).activity!]), ['300 CNY', '120 CNY']);
});
