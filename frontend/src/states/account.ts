import {createAsyncThunk, createSlice} from '@reduxjs/toolkit';
import {RootState} from '.';
import {fetcher} from '..';
import {Account, AccountStatus, LoadingState} from '../rest-model';
import AccountTrie from '../utils/AccountTrie';

export const fetchAccounts = createAsyncThunk('accounts/fetch', async (thunkApi) => {
  const ret = await fetcher(`/api/accounts`);
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
export const getAccountsTrie = (hideClosedAccount: boolean) => (state: RootState) => {
  const data = state.accounts.data;
  let trie = new AccountTrie();
  for (let account of data.filter((it) => (hideClosedAccount ? it.status === AccountStatus.Open : true))) {
    trie.insert(account);
  }
  return trie;
};


export const getAccountSelectItems = ()=> (state: RootState) => {
  return state.accounts.data.map(account => {
    const type = account.name.split(':')[0];
    return {
      label: account.name,
      value: account.name,
      group: type,
    };
  })
};