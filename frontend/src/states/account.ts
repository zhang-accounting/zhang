import { loadable_unwrap } from '.';
import { groupBy } from 'lodash-es';
import { atomWithRefresh, loadable } from 'jotai/utils';
import { atom } from 'jotai';
import { openAPIFetcher } from '../api/requests';


const findPetsByStatus = openAPIFetcher.path('/api/accounts').method('get').create()


export const accountFetcher = atomWithRefresh(async () => {
  return (await findPetsByStatus({})).data.data;
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
        items: groupedAccount[groupName].map((it) => ({
          value: it,
          label: it,
        })),
      }));
  });
});
