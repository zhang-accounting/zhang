import { Buffer } from 'buffer';

export function encodeBase64(str: string): string {
  return Buffer.from(str).toString('base64');
}

export function decodeBase64(str: string): string {
  return Buffer.from(str, 'base64').toString('utf-8');
}
