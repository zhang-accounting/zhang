import { base64Path } from '@/api/requests';

/** URL serving the raw document (images are previewed, everything else opens in a new tab). */
export function documentUrl(path: string) {
  return `/api/documents/${base64Path(path)}`;
}
