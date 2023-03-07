import { Chart, registerables } from 'chart.js';
import React from 'react';
import ReactDOM from 'react-dom';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { MantineProvider } from '@mantine/core';
import './i18n';
import axios from 'axios';
import { ModalsProvider } from "@mantine/modals";
import { DocumentPreviewModal } from "./components/modals/DocumentPreviewModal";
import { Provider } from 'react-redux';
import { store } from './states';
import { themeConfig } from './theme';

Chart.register(...registerables);
// @ts-ignore
export const fetcher = (...args) => axiosInstance.get(...args).then((res) => res.data.data);
const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';


if (development) {
    console.log("zhang is running in development mode");
}
export const serverBaseUrl = development ? 'http://localhost:8000' : "";
export const axiosInstance = axios.create({
    baseURL: serverBaseUrl,
    headers: {
        'Content-type': 'application/json',
    },
});

ReactDOM.render(
    <React.StrictMode>
        <Provider store={store}>
            <MantineProvider withGlobalStyles withNormalizeCSS theme={themeConfig}>
                <ModalsProvider modals={{ documentPreviewModal: DocumentPreviewModal }}>
                    <BrowserRouter>
                        <App></App>
                    </BrowserRouter>
                </ModalsProvider>
            </MantineProvider>
        </Provider>
    </React.StrictMode>,
    document.getElementById('root'),
);
