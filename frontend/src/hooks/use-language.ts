import { useLocalStorage } from '@/hooks/use-local-storage';
import { LANGUAGE_STORAGE_KEY } from '@/lib/languages';

/** `[lang, setLang]` persisted in localStorage (`lang`); App.tsx applies it to i18next. */
export function useLanguage() {
  return useLocalStorage({ key: LANGUAGE_STORAGE_KEY, defaultValue: 'en' });
}
