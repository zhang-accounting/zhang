import { Buffer } from 'buffer';
import { Document } from '@/api/types';

/** URL serving the raw document (images are previewed, everything else opens in a new tab). */
export function documentUrl(path: string) {
  return `/api/documents/${Buffer.from(path).toString('base64')}`;
}

export function documentExtension(document: Pick<Document, 'extension' | 'filename'>) {
  return (document.extension || document.filename.split('.').pop() || '').toUpperCase();
}
