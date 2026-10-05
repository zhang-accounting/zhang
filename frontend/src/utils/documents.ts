// Type-only imports keep this module runnable by `node --test` (documents.test.ts).
import type { Document } from '@/api/types';

/**
 * The image formats a browser shows in an `<img>` loaded from the download endpoint, by MIME type, with their file
 * extensions. The endpoint sends a document's bytes without a content type, which browsers sniff for these raster
 * formats. An SVG is shown only with its content type, and TIFF or HEIC not by every browser: such a document opens in a
 * new tab, like any other file.
 */
const PREVIEWABLE_IMAGES = new Map<string, string[]>([
  ['image/png', ['png']],
  ['image/jpeg', ['jpg', 'jpeg']],
  ['image/gif', ['gif']],
  ['image/webp', ['webp']],
  ['image/avif', ['avif']],
  ['image/bmp', ['bmp']],
]);

/** The type of a document as the documents lists show it: the extension of its file name, upper case (`PDF`). */
export function documentType(document: Pick<Document, 'extension'>): string {
  return (document.extension ?? '').toUpperCase();
}

/** Whether a document the server lists can be previewed as an image: by the MIME type the server gives it. */
export function canPreview(document: Pick<Document, 'mime_type'>): boolean {
  return document.mime_type != null && PREVIEWABLE_IMAGES.has(document.mime_type);
}

/**
 * Whether the document at `path` can be previewed as an image, for a path that comes without a MIME type: the `document`
 * metadata of a transaction in the journal preview. It reads the extension of the file name, for the formats of
 * {@link canPreview}.
 */
export function canPreviewPath(path: string): boolean {
  const name = path.split(/[/\\]/).pop() ?? '';
  const dot = name.lastIndexOf('.');
  const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
  return [...PREVIEWABLE_IMAGES.values()].some((extensions) => extensions.includes(extension));
}
