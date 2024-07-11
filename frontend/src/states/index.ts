import { configureStore } from '@reduxjs/toolkit';
import { TypedUseSelectorHook, useDispatch, useSelector } from 'react-redux';
import { Loadable } from 'jotai/vanilla/utils/loadable';

export const store = configureStore({
  reducer: {},
});

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;

export const useAppDispatch: () => AppDispatch = useDispatch;
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector;

export function loadable_unwrap<T, F>(val: Loadable<Promise<T>>, init_value: F, mapper: (data: T) => F): F {
  if (val.state === 'hasError' || val.state === 'loading') {
    return init_value;
  } else {
    return mapper(val.data as T);
  }
}
