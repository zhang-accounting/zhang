import { MantineProvider } from '@mantine/core';
import { ModalsProvider } from '@mantine/modals';
import { Notifications } from '@mantine/notifications';
import axios from 'axios';
import React from 'react';
import { createRoot } from 'react-dom/client';
import { Provider } from 'react-redux';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { TransactionPreviewModal } from './components/modals/TransactionPreviewModal';
import './i18n';
import { store } from './states';
import { themeConfig } from './theme';
import './global.css';
import '@mantine/core/styles.css';
import '@mantine/notifications/styles.css';
import '@mantine/dates/styles.css';
import '@mantine/charts/styles.css';
import '@mantine/dropzone/styles.css';
import { TransactionEditModal } from './components/modals/TransactionEditModal';
import { MantineEmotionProvider } from '@mantine/emotion';

// @ts-ignore
export const fetcher = <T,>(...args) => axiosInstance.get(...args).then((res) => res.data.data as T);
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
      <MantineProvider theme={themeConfig}>
        <MantineEmotionProvider>
          <ModalsProvider
            modals={{
              transactionPreviewModal: TransactionPreviewModal,
              transactionEditModal: TransactionEditModal,
            }}
          >
            <BrowserRouter>
              <Notifications />
              <App />
            </BrowserRouter>
          </ModalsProvider>
        </MantineEmotionProvider>
      </MantineProvider>
    </Provider>
  </React.StrictMode>,
);
