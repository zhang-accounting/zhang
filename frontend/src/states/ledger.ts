import { atom, useAtomValue } from 'jotai';
import { type DependencyList } from 'react';
// eslint-disable-next-line no-restricted-imports -- the one place that wraps react-use's async hooks
import { useAsyncRetry } from 'react-use';

/**
 * Bumped whenever the ledger may have changed: the server reloaded it (SSE `Reload`: an edit on disk, the raw editor,
 * another device, the midnight reload), or this app wrote to it. Every read of ledger data depends on it, the shared
 * atoms and `useLedgerQuery` alike, so one bump refreshes whatever is on screen.
 */
export const ledgerRevisionAtom = atom(0);
export const ledgerChangedAtom = atom(null, (get, set) => set(ledgerRevisionAtom, get(ledgerRevisionAtom) + 1));

/**
 * Runs `query`, an API call (`undefined` to wait), again whenever `deps` or the ledger change, and unwraps the
 * response's `data`. `firstLoad`: nothing to show yet; `refreshing`: the previous value stays on screen while a new one
 * loads.
 */
export function useLedgerQuery<T>(query: () => Promise<{ data: { data?: T } }> | undefined, deps: DependencyList) {
  const revision = useAtomValue(ledgerRevisionAtom);
  const state = useAsyncRetry(async () => (await query())?.data.data, [...deps, revision]);
  return { ...state, firstLoad: state.loading && state.value === undefined, refreshing: state.loading && state.value !== undefined };
}
