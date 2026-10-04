// The location of a ledger error (#493): its lines, never its byte offsets. Run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { errorLocation, spanLines } from './error-location.ts';

/** A translation that shows which text was chosen and with what. */
const t = (key: string, params: Record<string, string | number>) =>
  `${key}(${Object.entries(params)
    .map(([name, value]) => `${name}=${value}`)
    .join(',')})`;

/** The error of #493: a transaction on lines 5 to 7 of `main.zhang`, at bytes 98 to 157. */
const lunch = {
  filename: 'main.zhang',
  start: 98,
  end: 157,
  content: '2024-01-10 "Lunch"\n  Assets:A -10 CNY\n  Expenses:Food 5 CNY',
  line: 5,
  column: 1,
};

test('a directive spans the lines of its text from the line the server records', () => {
  assert.deepEqual(spanLines(lunch), { start: 5, end: 7 });
  // a newline ending the text adds no line; a one-line directive starts and ends on its line
  assert.deepEqual(spanLines({ ...lunch, content: `${lunch.content}\n` }), { start: 5, end: 7 });
  assert.deepEqual(spanLines({ content: '1970-01-01 open Assets:A CNY', line: 2 }), { start: 2, end: 2 });
  // a ledger with CRLF line endings
  assert.deepEqual(spanLines({ content: '2024-01-10 "Lunch"\r\n  Assets:A -10 CNY\r\n', line: 9 }), { start: 9, end: 10 });
});

test('the location is the file and its lines, not the byte offsets', () => {
  assert.equal(errorLocation(lunch, t), 'ERROR_BOX_LOCATION(file=main.zhang,start=5,end=7)');
  assert.equal(errorLocation({ ...lunch, content: '2024-01-10 "Lunch"' }, t), 'ERROR_BOX_LOCATION_LINE(file=main.zhang,line=5)');
});

test('without a line the location is only the file', () => {
  assert.equal(errorLocation({ ...lunch, line: null }, t), 'main.zhang');
  assert.equal(errorLocation({ ...lunch, line: undefined }, t), 'main.zhang');
  assert.equal(errorLocation({ content: '', line: null }, t), '');
  assert.equal(errorLocation(null, t), '');
});
