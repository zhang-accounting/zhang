import { configureStore } from "@reduxjs/toolkit"
import { TypedUseSelectorHook, useDispatch, useSelector } from "react-redux";
import { commoditiesSlice } from "./commodity";
import { errorsSlice } from './errors';




export const store = configureStore({
    reducer: {
        errors: errorsSlice.reducer,
        commodities: commoditiesSlice.reducer
    },
})

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;

export const useAppDispatch: () => AppDispatch = useDispatch
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector