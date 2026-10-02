import { Fetcher, type Middleware } from 'openapi-typescript-fetch';

import { paths } from './schemas';

// declare fetcher for paths
export const openAPIFetcher = Fetcher.for<paths>();

export const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';
export const backendUri: string = import.meta.env.VITE_API_ENDPOINT || 'http://localhost:8000';
export const serverBaseUrl = development ? backendUri : '';

/**
 * zhang-server reads list query params as `tags[]=a&tags[]=b`: `tags=a` (what the generated client writes for arrays) is a
 * 400, and a percent-encoded `tags%5B%5D=a` is silently ignored. Rewrite the list params of `/api/journals` / `/api/errors`.
 */
const LIST_PARAMS = new Set(['tags', 'links']);
const listQueryParams: Middleware = (url, init, next) => {
  const index = url.indexOf('?');
  if (index === -1) return next(url, init);
  const query = url
    .slice(index + 1)
    .split('&')
    .map((pair) => {
      const eq = pair.indexOf('=');
      const key = eq === -1 ? pair : pair.slice(0, eq);
      return LIST_PARAMS.has(decodeURIComponent(key)) ? `${key}[]${eq === -1 ? '' : pair.slice(eq)}` : pair;
    })
    .join('&');
  return next(`${url.slice(0, index)}?${query}`, init);
};

// global configuration
openAPIFetcher.configure({
  baseUrl: serverBaseUrl,
  init: {},
  use: [listQueryParams], // middlewares
});
