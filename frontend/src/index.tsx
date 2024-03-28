import { MantineProvider } from '@mantine/core';
import { ModalsProvider } from '@mantine/modals';
import { Notifications } from '@mantine/notifications';
import axios from 'axios';
import { Chart, registerables } from 'chart.js';
import React from 'react';
import { createRoot } from 'react-dom/client';
import { Provider } from 'react-redux';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { DocumentPreviewModal } from './components/modals/DocumentPreviewModal';
import { TransactionPreviewModal } from './components/modals/TransactionPreviewModal';
import './i18n';
import { store } from './states';
import { themeConfig } from './theme';

Chart.register(...registerables);
// @ts-ignore
export const fetcher = (...args) => axiosInstance.get(...args).then((res) => res.data.data);
const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';
const backendUri: string = process.env.REACT_APP_API_ENDPOINT || 'http://localhost:8000';

if (development) {
  console.log('zhang is running in development mode');
  console.log(`active backend is ${backendUri}`);
}
export const serverBaseUrl = development ? backendUri : '';
export const axiosInstance = axios.create({
  baseURL: serverBaseUrl,
  headers: {
    'Content-type': 'application/json',
  },
});

const container = document.getElementById('root');
const root = createRoot(container!);
root.render(
  <React.StrictMode>
    <Provider store={store}>
      <MantineProvider withGlobalStyles withNormalizeCSS theme={themeConfig}>
        <ModalsProvider
          modals={{
            documentPreviewModal: DocumentPreviewModal,
            transactionPreviewModal: TransactionPreviewModal,
          }}
        >
          <BrowserRouter>
            <Notifications />
            <App />
          </BrowserRouter>
        </ModalsProvider>
      </MantineProvider>
    </Provider>
  </React.StrictMode>,
);
