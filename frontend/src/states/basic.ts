import { PayloadAction, createAsyncThunk, createSlice } from '@reduxjs/toolkit';
import { axiosInstance, fetcher } from '..';
import { LoadingState } from '../rest-model';

export const fetchBasicInfo = createAsyncThunk('basic/fetch', async (thunkApi) => {
  const ret = await fetcher(`/api/info`);
  return ret;
});

export const reloadLedger = createAsyncThunk('basic/fetch', async (thunkApi) => {
  const ret = await axiosInstance.post('/api/reload');
  return ret;
});

interface BasicInfoState {
  title?: string;
  version?: string;
  isOnline: boolean;
  status: LoadingState;
  updatableVersion?: string;
}

const initialState: BasicInfoState = {
  title: undefined,
  version: undefined,
  isOnline: false,
  status: LoadingState.NotReady,
  updatableVersion: undefined,
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
    setUpdatableVersion: (state, action: PayloadAction<{ newVersion: string }>) => {
      state.updatableVersion = action.payload.newVersion;
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
