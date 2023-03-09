import { configureStore } from "@reduxjs/toolkit"
import { TypedUseSelectorHook, useDispatch, useSelector } from "react-redux";
import { accountsSlice } from "./account";
import { basicInfoSlice } from "./basic";
import { commoditiesSlice } from "./commodity";
import { errorsSlice } from './errors';




export const store = configureStore({
    reducer: {
        basic: basicInfoSlice.reducer,
        errors: errorsSlice.reducer,
        commodities: commoditiesSlice.reducer,
        accounts: accountsSlice.reducer
    },
})

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;

export const useAppDispatch: () => AppDispatch = useDispatch
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector