import { Fetcher } from 'openapi-typescript-fetch';

import { paths } from './schemas';

// declare fetcher for paths
export const openAPIFetcher = Fetcher.for<paths>();

export const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';
export const backendUri: string = import.meta.env.VITE_API_ENDPOINT || 'http://localhost:8000';
export const serverBaseUrl = development ? backendUri : '';

// global configuration
openAPIFetcher.configure({
  baseUrl: serverBaseUrl,
  init: {},
  use: [], // middlewares
});
