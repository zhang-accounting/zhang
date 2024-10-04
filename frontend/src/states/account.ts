import { loadable_unwrap } from '.';
import { fetcher } from '../global.ts';
import { Account } from '../rest-model';
import { groupBy } from 'lodash-es';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { atom } from 'jotai';

export const accountFetcher = atomWithRefresh(async (get) => {
  return await fetcher<Account[]>(`/api/accounts`);
});

export const accountAtom = loadable(accountFetcher);

export const accountSelectItemsAtom = atom((get) => {
  return loadable_unwrap(get(accountAtom), [], (data) => {
    const groupedAccount = groupBy(
      data.map((account) => account.name),
      (it) => it.split(':')[0],
    );
    return Object.keys(groupedAccount)
      .sort()
      .map((groupName) => ({
        group: groupName,
        items: groupedAccount[groupName],
      }));
  });
});
