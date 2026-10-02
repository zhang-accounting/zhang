import { useLocalStorage } from '@/hooks/use-local-storage';

import { useTheme } from 'next-themes';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Router } from './router';
import { Toaster } from './components/ui/sonner';
import { AppShell } from './layout/AppShell';
import { AuthGate } from './layout/AuthGate';
import { useServerEvents } from './layout/use-server-events';

/** `--background` in light / dark (see global.css); used for `<meta name="theme-color">` (browser chrome, PWA title bar). */
const THEME_COLOR = { light: '#f9fafa', dark: '#111312' };

/** BCP 47 tag for `<html lang>` (screen-reader pronunciation, CJK font selection, hyphenation). */
const htmlLang = (language: string | undefined) => (language?.startsWith('zh') ? 'zh-CN' : 'en');

export default function App() {
  const { i18n } = useTranslation();
  const { resolvedTheme } = useTheme();
  const [lang] = useLocalStorage({ key: 'lang', defaultValue: 'en' });

  useEffect(() => {
    if (i18n.language !== lang) {
      i18n.changeLanguage(lang);
    }
  }, [i18n, lang]);

  // index.html ships one theme-color per OS scheme; once the app knows the chosen theme (which may differ from the OS), follow it.
  useEffect(() => {
    if (!resolvedTheme) return;
    const color = resolvedTheme === 'dark' ? THEME_COLOR.dark : THEME_COLOR.light;
    document.querySelectorAll('meta[name="theme-color"]').forEach((meta) => meta.setAttribute('content', color));
  }, [resolvedTheme]);

  useEffect(() => {
    const apply = (language: string) => {
      document.documentElement.lang = htmlLang(language);
    };
    apply(i18n.language);
    i18n.on('languageChanged', apply);
    return () => i18n.off('languageChanged', apply);
  }, [i18n]);

  return (
    <AuthGate>
      <LedgerApp />
    </AuthGate>
  );
}

/**
 * The signed-in app: shell, routes, the SSE stream and the toasts (mounted by `AuthGate` only while access is granted, so the
 * error toasts of requests cut off by an expired session do not cover the login page; it dismisses them on sign-in).
 */
function LedgerApp() {
  useServerEvents();
  return (
    <>
      <AppShell>
        <Router />
      </AppShell>
      <Toaster mobileOffset={{ bottom: 'calc(4.5rem + env(safe-area-inset-bottom))' }} />
    </>
  );
}
