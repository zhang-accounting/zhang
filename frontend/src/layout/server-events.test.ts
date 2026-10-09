// Tests of the SSE event handling, run with Node's built-in runner (Node >= 23.6 strips the types): pnpm run test
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { serverEventHandler, type ServerEventActions } from './server-events.ts';

/** A handler whose actions record what they were called with, in order. */
function recording() {
  const calls: string[] = [];
  const actions: ServerEventActions = {
    ledgerChanged: () => calls.push('ledgerChanged'),
    reloaded: () => calls.push('reloaded'),
    reloadFailed: (failure) => calls.push(`reloadFailed:${failure.message}`),
    connected: (afterOutage) => calls.push(`connected:${afterOutage}`),
    newVersion: (version) => calls.push(`newVersion:${version}`),
    disconnected: (closed) => calls.push(`disconnected:${closed}`),
  };
  return { calls, handler: serverEventHandler(actions) };
}

test('the Connected of a page load is not a ledger change', () => {
  const { calls, handler } = recording();
  handler.message({ type: 'Connected' });
  assert.deepEqual(calls, ['connected:false']);
});

test('a reconnect after an outage is one ledger change, as the reloads broadcast meanwhile were lost', () => {
  const { calls, handler } = recording();
  handler.message({ type: 'Connected' });
  // the server went away: EventSource retries by itself and fires onerror on every failed attempt
  handler.error(false);
  handler.error(false);
  calls.length = 0;

  handler.message({ type: 'Connected' });
  assert.deepEqual(calls, ['connected:true', 'ledgerChanged']);

  // the connection stays up: a later Connected (none is expected, but the server may send one) is not an outage's end
  handler.message({ type: 'Connected' });
  assert.deepEqual(calls, ['connected:true', 'ledgerChanged', 'connected:false']);
});

test('a reload is a ledger change, a failed reload is not', () => {
  const { calls, handler } = recording();
  handler.message({ type: 'Reload' });
  handler.message({ type: 'ReloadFailed', file: 'main.zhang', message: 'cannot parse main.zhang' });
  assert.deepEqual(calls, ['reloaded', 'ledgerChanged', 'reloadFailed:cannot parse main.zhang']);
});

test('a stream refused for good is reported as closed, and other events pass through', () => {
  const { calls, handler } = recording();
  handler.error(true);
  handler.message({ type: 'NewVersionFound', version: '1.2.3' });
  handler.message(undefined);
  handler.message({ type: 'Unknown' } as never);
  assert.deepEqual(calls, ['disconnected:true', 'newVersion:1.2.3']);
});
