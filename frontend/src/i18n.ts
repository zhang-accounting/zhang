import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import Backend from 'i18next-http-backend';

// eslint-disable-line
const isDevelopment = process.env.NODE_ENV === 'development';
i18n
  .use(Backend)
  .use(initReactI18next)
  .init({
    lng: 'en',
    fallbackLng: false,
    saveMissing: isDevelopment,
    debug: isDevelopment,
    returnEmptyString: false,
    interpolation: {
      escapeValue: false, // not needed for react as it escapes by default
    },
    backend: {
      loadPath: '/locales/{{lng}}/{{ns}}.json',
      addPath: 'http://127.0.0.1:3001/locales/add/{{lng}}/{{ns}}.json',
    },
    detection: {
      order: ['path', 'querystring', 'cookie'],
      caches: ['cookie'],
      cookieMinutes: 160,
      lookupQuerystring: 'lang',
      lookupFromPathIndex: 0,
    },
    react: {
      useSuspense: false,
    },
  });

export default i18n;
