import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import './i18n';
import { themeConfig } from './theme';
import './global.css';
import '@mantine/core/styles.css';
import '@mantine/notifications/styles.css';
import '@mantine/dates/styles.css';
import '@mantine/charts/styles.css';
import '@mantine/dropzone/styles.css';
import { TooltipProvider } from './components/ui/tooltip';
import { Toaster } from './components/ui/sonner';
import { MantineProvider } from '@mantine/core';
import { MantineEmotionProvider } from '@mantine/emotion';


createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <TooltipProvider>

    
    <MantineProvider>
      <MantineEmotionProvider>
        
          <BrowserRouter>
            <App />
            <Toaster />
          </BrowserRouter>
      </MantineEmotionProvider>
    </MantineProvider>
    </TooltipProvider>
  </React.StrictMode>,
);