import assert from 'node:assert/strict';
import { test } from 'node:test';
import { tokenKind, tokenRegexp } from './bql-tokens.ts';

/** The keywords of a part of the parser, as `/api/query/schema` lists them. */
const KEYWORDS = ['select', 'from', 'where', 'as', 'and', 'case', 'when', 'then', 'else', 'end', 'null'];

function tokens(query: string, keywords: readonly string[] = KEYWORDS) {
  return [...query.matchAll(tokenRegexp(keywords))].map((match) => [match[0], tokenKind(match as RegExpExecArray)]);
}

test('the keywords the server lists are keywords, CASE and WHEN included', () => {
  assert.deepEqual(tokens("SELECT CASE WHEN number > 0 THEN 'in' ELSE NULL END AS side FROM #budgets"), [
    ['SELECT', 'keyword'],
    ['CASE', 'keyword'],
    ['WHEN', 'keyword'],
    ['0', 'number'],
    ['THEN', 'keyword'],
    ["'in'", 'string'],
    ['ELSE', 'keyword'],
    ['NULL', 'keyword'],
    ['END', 'keyword'],
    ['AS', 'keyword'],
    ['FROM', 'keyword'],
    ['#budgets', 'table'],
  ]);
});

test('a word is no keyword before the server lists it, and a function call stays one', () => {
  assert.deepEqual(tokens('select sum(number)', []), [['sum', 'function']]);
  assert.deepEqual(tokens("select date > 2024-01-01 and account ~ 'Food'"), [
    ['select', 'keyword'],
    ['2024-01-01', 'date'],
    ['and', 'keyword'],
    ["'Food'", 'string'],
  ]);
});
