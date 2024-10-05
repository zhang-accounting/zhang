import { MantineProvider } from '@mantine/core';
import { ModalsProvider } from '@mantine/modals';
import { Notifications } from '@mantine/notifications';
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { TransactionPreviewModal } from './components/modals/TransactionPreviewModal';
import './i18n';
import { themeConfig } from './theme';
import './global.css';
import '@mantine/core/styles.css';
import '@mantine/notifications/styles.css';
import '@mantine/dates/styles.css';
import '@mantine/charts/styles.css';
import '@mantine/dropzone/styles.css';
import { TransactionEditModal } from './components/modals/TransactionEditModal';
import { MantineEmotionProvider } from '@mantine/emotion';
import { TooltipProvider } from './components/ui/tooltip';


createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <TooltipProvider>

    
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
    </TooltipProvider>
  </React.StrictMode>,
);