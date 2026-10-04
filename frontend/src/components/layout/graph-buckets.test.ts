// Pure-function tests for the labels of the report graph's buckets, run with Node's built-in runner:
//   pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { labelDay } from './graph-buckets.ts';

test('a week or month that starts before the range is labelled by the first day of the range', () => {
  // the range starts on Saturday, March 1; its first week starts on Monday, February 24
  const from = new Date(2025, 2, 1);
  assert.deepEqual(labelDay(new Date(2025, 1, 24), from), from);
  // a month that starts in the middle of the range
  assert.deepEqual(labelDay(new Date(2025, 0, 1), new Date(2025, 0, 15)), new Date(2025, 0, 15));
});

test('the other buckets are labelled by their first day', () => {
  const from = new Date(2025, 2, 1);
  assert.deepEqual(labelDay(new Date(2025, 2, 3), from), new Date(2025, 2, 3));
  assert.deepEqual(labelDay(from, from), from);
});
