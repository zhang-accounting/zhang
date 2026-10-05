// Tests of the transaction form's server preview handling, run with Node's built-in runner: pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  createPreviewer,
  fieldErrors,
  ledgerErrors,
  type PreviewState,
  previewKey,
  refused,
  type TransactionPreview,
  unbalancedText,
} from './transaction-preview.ts';

function preview(fields: Partial<TransactionPreview>): TransactionPreview {
  return { text: null, field_errors: [], unbalanced: [], errors: [], ...fields };
}

const KEY = previewKey({ payee: 'Broker', postings: [{ account: 'Assets:Cash', unit: '10 AAPL {150 USD}' }] });

const REFUSED: PreviewState = {
  key: KEY,
  preview: preview({
    unbalanced: null,
    field_errors: [
      { posting: 0, field: 'unit', message: 'invalid amount "10 AAPL {150 USD}"' },
      { posting: 1, field: 'cost', message: 'invalid cost "150 USD"' },
      { posting: 1, field: 'price', message: 'invalid price "6 USD"' },
      { posting: 1, field: 'cost', message: 'a second cost message' },
      { posting: null, field: 'tags', message: 'invalid tag "two words"' },
    ],
  }),
};

test('fieldErrors gives the first message of each field of a posting, or of the transaction', () => {
  assert.deepEqual(fieldErrors(REFUSED, KEY, 0), { unit: 'invalid amount "10 AAPL {150 USD}"' });
  assert.deepEqual(fieldErrors(REFUSED, KEY, 1), { cost: 'invalid cost "150 USD"', price: 'invalid price "6 USD"' });
  assert.deepEqual(fieldErrors(REFUSED, KEY, 2), {});
  assert.deepEqual(fieldErrors(REFUSED, KEY, null), { tags: 'invalid tag "two words"' });
});

test('the field errors of an older request are not shown, nor do they block saving', () => {
  const edited = previewKey({ payee: 'Broker', postings: [{ account: 'Assets:Cash', unit: '10 AAPL' }] });
  assert.deepEqual(fieldErrors(REFUSED, edited, 0), {});
  assert.equal(refused(REFUSED, KEY), true);
  assert.equal(refused(REFUSED, edited), false);
  assert.equal(refused(undefined, KEY), false);
  // a failed preview request blocks nothing: saving tells the server's answer
  assert.equal(refused({ key: KEY, error: 'network error' }, KEY), false);
  assert.equal(refused({ key: KEY, preview: preview({ text: '2024-01-15 * "Broker"' }) }, KEY), false);
});

test('unbalancedText lists what the server found the postings unbalanced by, at the precision it rounded them to', () => {
  // a stock purchase weighed by its cost balances: nothing to warn about
  assert.equal(unbalancedText(preview({ unbalanced: [] })), null);
  assert.equal(unbalancedText(preview({ unbalanced: null })), null);
  assert.equal(unbalancedText(undefined), null);
  assert.equal(
    unbalancedText(
      preview({
        unbalanced: [
          { number: '0.50', commodity: 'CNY' },
          { number: '-1', commodity: 'USD' },
        ],
      }),
    ),
    '0.50 CNY, -1 USD',
  );
});

test('ledgerErrors lists the errors the ledger would report but the imbalance, which has its own line', () => {
  const closed = { error_type: 'AccountClosed' as const, metas: { account_name: 'Expenses:Food' } };
  assert.deepEqual(ledgerErrors(preview({ errors: [{ error_type: 'UnbalancedTransaction', metas: {} }, closed] })), [closed]);
  assert.deepEqual(ledgerErrors(undefined), []);
});

/** Lets the pending promise callbacks run. */
async function settle() {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

test('createPreviewer asks once, after the last change, and keeps only the newest answer', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const answers: PreviewState[] = [];
  const previewer = createPreviewer((state) => answers.push(state), 300);
  const asked: string[] = [];
  const pending: Record<string, (preview: TransactionPreview) => void> = {};
  const fetch = (key: string) => () => {
    asked.push(key);
    return new Promise<TransactionPreview>((resolve) => (pending[key] = resolve));
  };
  const describe = async () => 'failed';

  previewer.request('a', fetch('a'), describe);
  t.mock.timers.tick(100);
  previewer.request('ab', fetch('ab'), describe);
  t.mock.timers.tick(299);
  assert.deepEqual(asked, [], 'nothing is asked while typing');
  t.mock.timers.tick(1);
  assert.deepEqual(asked, ['ab'], 'only the latest request is asked');

  // a slow answer to an older request comes after the answer to a newer one: it is dropped
  previewer.request('abc', fetch('abc'), describe);
  t.mock.timers.tick(300);
  pending.abc(preview({ text: 'abc' }));
  await settle();
  pending.ab(preview({ text: 'ab' }));
  await settle();
  assert.deepEqual(
    answers.map((it) => it.key),
    ['abc'],
  );
  assert.equal(answers[0].preview?.text, 'abc');
});

test('createPreviewer reports a failed request with its message, and nothing once cancelled', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const answers: PreviewState[] = [];
  const previewer = createPreviewer((state) => answers.push(state), 300);
  previewer.request(
    'a',
    () => Promise.reject(new Error('404')),
    async (error) => `message of ${(error as Error).message}`,
  );
  t.mock.timers.tick(300);
  await settle();
  assert.deepEqual(answers, [{ key: 'a', error: 'message of 404' }]);

  previewer.request(
    'b',
    () => Promise.resolve(preview({})),
    async () => '',
  );
  previewer.cancel();
  t.mock.timers.tick(300);
  await settle();
  assert.equal(answers.length, 1);
});
