import { createAsyncThunk, createSlice, PayloadAction } from '@reduxjs/toolkit';
import { RootState } from '.';
import { fetcher } from '..';
import { LoadingState } from '../rest-model';

export const fetchJournals = createAsyncThunk('journals/fetch', async (keyword: string, { getState }) => {
  const current_page = (getState() as RootState).journals.current_page;
  const url = keyword.trim() === '' ? `/api/journals?page=${current_page}` : `/api/journals?page=${current_page}&keyword=${keyword.trim()}`;
  const ret = await fetcher(url);
  return ret;
});

interface JournalState {
  total_number: number;
  total_page: number;
  current_page: number;
  items: any[];
  status: LoadingState;
}

const initialState: JournalState = {
  total_number: 0,
  total_page: 1,
  current_page: 1,
  items: [],
  status: LoadingState.NotReady,
};

export const journalsSlice = createSlice({
  name: 'journals',
  initialState,
  reducers: {
    setPage: (state, action: PayloadAction<{ current_page: number }>) => {
      state.current_page = action.payload.current_page;
    },
    clear: (state) => {
      state.status = LoadingState.NotReady;
    },
  },
  extraReducers: (builder) => {
    builder.addCase(fetchJournals.pending, (state, action) => {
      state.status = LoadingState.Loading;
    });

    builder.addCase(fetchJournals.fulfilled, (state, action) => {
      state.status = LoadingState.Success;
      state.total_number = action.payload.total_count;
      state.total_page = action.payload.total_page;
      state.items = action.payload.records;
    });
  },
});
