import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router';
import { ThemeProvider } from 'next-themes';
import App from './App';
import './i18n';
import './global.css';
import { TooltipProvider } from './components/ui/tooltip';

// SPA only: next-themes' inline script never runs when React renders it on the client (and React 19 warns about it),
// so mark it as a data block. index.html applies the persisted theme class before first paint instead.
const THEME_SCRIPT_PROPS = { type: 'application/json' };

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange scriptProps={THEME_SCRIPT_PROPS}>
      <TooltipProvider>
        <BrowserRouter>
          <App />
        </BrowserRouter>
      </TooltipProvider>
    </ThemeProvider>
  </React.StrictMode>,
);
