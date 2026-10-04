// The Raw Editing deep link the error list opens a file with (#493), run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { lineFromSearch, rawEditLink } from './raw-edit-link.ts';

test('an error opens its file in the editor at the line of its directive', () => {
  assert.equal(rawEditLink('main.zhang', 5), '/edit?file=main.zhang&line=5');
  // the file is a search parameter: a nested path with spaces and an ampersand survives the round trip
  const link = rawEditLink('data/2024 Q1/food & drink.zhang', 12);
  assert.equal(link, '/edit?file=data%2F2024+Q1%2Ffood+%26+drink.zhang&line=12');
  const params = new URL(link ?? '', 'http://localhost').searchParams;
  assert.equal(params.get('file'), 'data/2024 Q1/food & drink.zhang');
  assert.equal(lineFromSearch(params), 12);
});

test('without a line the file opens where the editor starts; without a file there is nothing to open', () => {
  assert.equal(rawEditLink('main.zhang', null), '/edit?file=main.zhang');
  assert.equal(rawEditLink('main.zhang', undefined), '/edit?file=main.zhang');
  assert.equal(rawEditLink(null, 5), null);
  assert.equal(rawEditLink(undefined, 5), null);
  assert.equal(rawEditLink('', 5), null);
});

test('only a positive whole number is a line to go to', () => {
  assert.equal(lineFromSearch('?file=main.zhang&line=7'), 7);
  assert.equal(lineFromSearch('?file=main.zhang'), null);
  assert.equal(lineFromSearch(''), null);
  assert.equal(lineFromSearch('?line='), null);
  assert.equal(lineFromSearch('?line=0'), null);
  assert.equal(lineFromSearch('?line=-3'), null);
  assert.equal(lineFromSearch('?line=2.5'), null);
  assert.equal(lineFromSearch('?line=abc'), null);
});
