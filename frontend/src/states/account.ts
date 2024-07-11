import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';
import { RootState } from '.';
import { fetcher } from '..';
import { Account, AccountStatus, LoadingState } from '../rest-model';
import AccountTrie from '../utils/AccountTrie';
import { groupBy } from 'lodash-es';

export const fetchAccounts = createAsyncThunk('accounts/fetch', async (thunkApi) => {
  const ret = await fetcher<any>(`/api/accounts`);
  return ret;
});

interface AccountsState {
  dataMap: { [accountName: string]: Account };
  data: Account[];
  status: LoadingState;
}

const initialState: AccountsState = {
  dataMap: {},
  data: [],
  status: LoadingState.NotReady,
};

export const accountsSlice = createSlice({
  name: 'accounts',
  initialState,
  reducers: {
    clear: (state) => {
      state.status = LoadingState.NotReady;
    },
  },
  extraReducers: (builder) => {
    builder.addCase(fetchAccounts.pending, (state, action) => {
      state.status = LoadingState.Loading;
    });

    builder.addCase(fetchAccounts.fulfilled, (state, action) => {
      state.status = LoadingState.Success;
      state.dataMap = Object.fromEntries(action.payload.map((item: Account) => [item.name, item]));
      state.data = action.payload;
    });
  },
});

export const getAccountByName = (name: string) => (state: RootState) => state.accounts.dataMap[name];
export const getAccountsTrie = (hideClosedAccount: boolean, filterKeyword: string) => (state: RootState) => {
  const data = state.accounts.data;
  let trie = new AccountTrie();
  for (let account of data.filter((it) => (hideClosedAccount ? it.status === AccountStatus.Open : true))) {
    let trimmedKeyword = filterKeyword.trim();
    if (trimmedKeyword !== '') {
      if (account.name.toLowerCase().includes(trimmedKeyword.toLowerCase()) || (account.alias?.toLowerCase() ?? '').includes(trimmedKeyword.toLowerCase())) {
        trie.insert(account);
      }
    } else {
      trie.insert(account);
    }
  }
  return trie;
};

export const getAccountSelectItems = () => (state: RootState) => {
  const groupedAccount = groupBy(
    state.accounts.data.map((account) => account.name),
    (it) => it.split(':')[0],
  );
  return Object.keys(groupedAccount)
    .sort()
    .map((groupName) => ({
      group: groupName,
      items: groupedAccount[groupName],
    }));
};
