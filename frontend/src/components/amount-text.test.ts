// An amount written as text the way `Amount` shows it (audit D12), run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import BigNumber from 'bignumber.js';
import { amountTextParts, formatAmountText } from './amount-text.ts';

const cny = { precision: 2, prefix: '¥', suffix: null };
const usd = { precision: 2, prefix: null, suffix: null };

test('an amount is written at its commodity precision, with its prefix', () => {
  // the sidebar tooltip wrote `1,234.5678 CNY` next to a row showing `¥1,234.57`
  assert.equal(formatAmountText(new BigNumber('1234.5678'), 'CNY', cny), '¥1,234.57');
  assert.equal(formatAmountText('1234.5', 'CNY', cny), '¥1,234.50');
  assert.equal(formatAmountText('1234.5678', 'JPY', { precision: 0, prefix: '¥' }), '¥1,235');
});

test('a commodity without a prefix or a suffix is named after the number', () => {
  assert.equal(formatAmountText('7', 'USD', usd), '7.00 USD');
  assert.equal(formatAmountText('12.5', 'PTS', { precision: 1, suffix: ' pts' }), '12.5 pts');
});

test('the sign comes first, before the prefix', () => {
  assert.equal(formatAmountText('-1234.5', 'CNY', cny), '-¥1,234.50');
  assert.equal(formatAmountText('5', 'CNY', cny, { signed: true }), '+¥5.00');
  assert.equal(formatAmountText('0', 'CNY', cny, { signed: true }), '¥0.00');
});

test('exact keeps every decimal, plain drops the commodity', () => {
  assert.equal(formatAmountText('0.001', 'USD', usd), '0.00 USD');
  assert.equal(formatAmountText('0.001', 'USD', usd, { exact: true }), '0.001 USD');
  assert.equal(formatAmountText('3', 'USD', usd, { exact: true }), '3.00 USD');
  assert.equal(formatAmountText('1234.5678', 'CNY', cny, { plain: true }), '1,234.57');
});

test('a commodity the ledger does not declare takes the default precision', () => {
  assert.equal(formatAmountText('1234.5678', 'XYZ', undefined), '1,234.57 XYZ');
  assert.equal(formatAmountText('1234.5678', 'XYZ', undefined, { defaultPrecision: 3 }), '1,234.568 XYZ');
  // a declared commodity keeps its own precision
  assert.equal(formatAmountText('1234.5678', 'CNY', cny, { defaultPrecision: 3 }), '¥1,234.57');
});

test('the parts are those Amount shows', () => {
  assert.deepEqual(amountTextParts('-1234.5', 'CNY', cny), { sign: '-', prefix: '¥', number: '1,234.50', suffix: '', currency: undefined });
  assert.deepEqual(amountTextParts('7', 'USD', usd, { signed: true }), { sign: '+', prefix: '', number: '7.00', suffix: '', currency: 'USD' });
  assert.deepEqual(amountTextParts('7', 'USD', usd, { plain: true }), { sign: '', prefix: '', number: '7.00', suffix: '', currency: undefined });
});
