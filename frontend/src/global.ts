// @ts-ignore
import axios from 'axios';

export const fetcher = <T, >(...args) => axiosInstance.get(...args).then((res) => res.data.data as T);
export const development: boolean = !process.env.NODE_ENV || process.env.NODE_ENV === 'development';
export const backendUri: string = import.meta.env.VITE_API_ENDPOINT || 'http://localhost:8000';
export const serverBaseUrl = development ? backendUri : '';
export const axiosInstance = axios.create({
  baseURL: serverBaseUrl,
  headers: {
    'Content-type': 'application/json',
  },
});
if (development) {
  console.log('zhang is running in development mode');
  console.log(`active backend is ${backendUri}`);
}