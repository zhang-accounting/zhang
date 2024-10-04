import { loadable_unwrap } from '.';
import { fetcher } from '../global.ts';
import { CommodityListItem } from '../rest-model';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { groupBy } from 'lodash-es';
import { atom } from 'jotai';

export const FRONTEND_DEFAULT_GROUP = '__ZHANG__FRONTEND_DEFAULT__GROUP__';

export const commoditiesFetcher = atomWithRefresh(async (get) => {
  const ret = await fetcher<CommodityListItem[]>(`/api/commodities`);
  return Object.fromEntries(ret.map((item: CommodityListItem) => [item.name, item]));
});

export const commoditiesAtom = loadable(commoditiesFetcher);

export const groupedCommoditiesAtom = atom((get) => {
  return loadable_unwrap(get(commoditiesAtom), {}, (data) => groupBy(data, (it) => it.group ?? FRONTEND_DEFAULT_GROUP));
});
