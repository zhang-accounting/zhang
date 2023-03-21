import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';
import { fetcher } from '..';
import { LoadingState } from '../rest-model';

export const fetchBasicInfo = createAsyncThunk('basic/fetch', async (thunkApi) => {
  const ret = await fetcher(`/api/info`);
  return ret;
});

interface BasicInfoState {
  title?: String;
  version?: String;
  isOnline: boolean;
  status: LoadingState;
}

const initialState: BasicInfoState = {
  title: undefined,
  version: undefined,
  isOnline: false,
  status: LoadingState.NotReady,
};

export const basicInfoSlice = createSlice({
  name: 'basic',
  initialState,
  reducers: {
    online: (state) => {
      state.isOnline = true;
    },
    offline: (state) => {
      state.isOnline = false;
    },
  },
  extraReducers: (builder) => {
    builder.addCase(fetchBasicInfo.pending, (state, action) => {
      state.status = LoadingState.Loading;
    });

    builder.addCase(fetchBasicInfo.fulfilled, (state, action) => {
      state.status = LoadingState.Success;
      state.isOnline = true;
      state.title = action.payload.title;
      state.version = action.payload.version;
    });
  },
});
