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
        fallbackLng: 'en',
        debug: isDevelopment,
        interpolation: {
            escapeValue: false, // not needed for react as it escapes by default
        },
        react:{
            useSuspense: false
        }
    });


export default i18n;