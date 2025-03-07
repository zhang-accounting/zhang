import { loadable_unwrap } from '.';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { groupBy } from 'lodash-es';
import { atom } from 'jotai';
import { openAPIFetcher } from '../api/fetcher';

export const FRONTEND_DEFAULT_GROUP = '__ZHANG__FRONTEND_DEFAULT__GROUP__';

const findCommodities = openAPIFetcher.path('/api/commodities').method('get').create()

export const commoditiesFetcher = atomWithRefresh(async () => {
  const ret = (await findCommodities({})).data.data;
  return Object.fromEntries(ret.map((item) => [item.name, item]));
});

export const commoditiesAtom = loadable(commoditiesFetcher);

export const groupedCommoditiesAtom = atom((get) => {
  return loadable_unwrap(get(commoditiesAtom), {}, (data) => groupBy(data, (it) => it.group ?? FRONTEND_DEFAULT_GROUP));
});
