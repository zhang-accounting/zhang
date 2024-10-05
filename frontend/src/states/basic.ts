import { fetcher } from '../global.ts';
import { atom } from 'jotai';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { loadable_unwrap } from './index';

export const onlineAtom = atom<boolean>(false);
export const updatableVersionAtom = atom<string | undefined>(undefined);

export const basicInfoFetcher = atomWithRefresh(async (get) => {
  return await fetcher<{ title: string; version: string }>(`/api/info`);
});

export const basicInfoAtom = loadable(basicInfoFetcher);

export const titleAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), 'Zhang Accounting', (data) => data.title);
});
export const versionAtom = atom((get) => {
  return loadable_unwrap(get(basicInfoAtom), undefined, (data) => data.version);
});

export const breadcrumbAtom = atom<{label: string, uri: string}[]>([]);