import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';
import { RootState } from '.';
import { fetcher } from '..';
import { CommodityListItem, LoadingState } from '../rest-model';

export const fetchCommodities = createAsyncThunk('commodities/fetch', async (thunkApi) => {
  const ret = await fetcher<any>(`/api/commodities`);
  return ret;
});

interface CommoditiesState {
  value: { [key: string]: CommodityListItem };
  status: LoadingState;
}

const initialState: CommoditiesState = {
  value: {},
  status: LoadingState.NotReady,
};

export const commoditiesSlice = createSlice({
  name: 'commodities',
  initialState,
  reducers: {},
  extraReducers: (builder) => {
    builder.addCase(fetchCommodities.pending, (state, action) => {
      state.status = LoadingState.Loading;
    });

    builder.addCase(fetchCommodities.fulfilled, (state, action) => {
      state.status = LoadingState.Success;
      state.value = Object.fromEntries(action.payload.map((item: CommodityListItem) => [item.name, item]));
    });
  },
});

export const getCommodityByName = (name: string) => (state: RootState) => state.commodities.value[name];
