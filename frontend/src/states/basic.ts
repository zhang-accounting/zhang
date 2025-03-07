import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { openAPIFetcher } from '../api/fetcher.ts';
import { loadable_unwrap } from './index';

const fetchBaseInfo = openAPIFetcher.path('/api/info').method('get').create()


export const onlineAtom = atom<boolean>(false);
export const updatableVersionAtom = atom<string | undefined>(undefined);

export const basicInfoFetcher = atomWithRefresh(async () => {
  return (await fetchBaseInfo({})).data.data;
});

export const basicInfoAtom = loadable(basicInfoFetcher);

export const titleAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), 'Zhang Accounting', (data) => data.title);
});
export const versionAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), undefined, (data) => data.version);
});

export const breadcrumbAtom = atom<{ label: string; uri: string; noTranslate?: boolean }[]>([]);
