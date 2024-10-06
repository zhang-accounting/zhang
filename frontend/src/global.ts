import axios from 'axios';
import { t } from 'i18next';
import { toast } from 'sonner';

// @ts-ignore TS2366
export const fetcher = async <T>(...args: any): Promise<T> => {
  try {
    // @ts-ignore TS2556
    const res = await axiosInstance.get(...args);
    return res.data.data as T;
  } catch (error: unknown) {
    if (axios.isAxiosError(error)) {
      toast.error(t('REQUEST_FAILED'), {
        description: error?.response?.data ?? 'failed to send request',
      });
    }
    // throw error;
    console.error(error);
  }
};

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
