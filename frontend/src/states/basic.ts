import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { openAPIFetcher } from '../api/fetcher.ts';
import { loadable_unwrap } from './index';
import { ledgerRevisionAtom } from './ledger';

const fetchBaseInfo = openAPIFetcher.path('/api/info').method('get').create();

export const onlineAtom = atom<boolean>(false);
export const updatableVersionAtom = atom<string | undefined>(undefined);

/** Read again with every ledger revision; also refreshed on its own by SSE `ReloadFailed` and `Connected`, which change no ledger data. */
export const basicInfoFetcher = atomWithRefresh(async (get) => {
  get(ledgerRevisionAtom);
  return (await fetchBaseInfo({})).data.data;
});

export const basicInfoAtom = loadable(basicInfoFetcher);

export const titleAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), 'Zhang Accounting', (data) => data.title);
});
export const versionAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), undefined, (data) => data.version);
});
/**
 * The failure of the last reload, while the server still serves the ledger loaded before it (`reload_failure` of `/api/info`,
 * refreshed on the SSE `ReloadFailed` and `Reload` events); `undefined` once a reload succeeded.
 */
export const reloadFailureAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), undefined, (data) => data.reload_failure ?? undefined);
});

export const breadcrumbAtom = atom<{ label: string; uri: string; noTranslate?: boolean }[]>([]);
