// The documents lists' items from the rows of the built-in queries `journals.documents` and `accounts.documents`, and
// the helpers the lists render with. Type-only imports keep this module runnable by `node --test` (documents.test.ts).
import type { Builtins } from '@/api/builtins';

/** A row of `journals.documents` or `accounts.documents`: the same columns. */
export type DocumentRow = Builtins['journals.documents']['row'];

/** A document as the lists show it. */
export interface Document {
  /** the ledger's wall-clock date and time of the directive or transaction, `YYYY-MM-DDTHH:MM:SS` */
  datetime: string;
  /** the file name of `path` */
  filename: string;
  /** the path of the file, relative to the ledger's directory: what the download endpoint takes */
  path: string;
  /** the extension of the file name, lower case and without the dot (`pdf`); `null` for a file name without one */
  extension: string | null;
  /** the account of a `document` directive, or of the posting whose metadata names the file */
  account: string | null;
  /** the transaction whose metadata, or whose posting's, names the file */
  trx_id: string | null;
}

/** The file name of a path, after its last `/` or `\`. */
function fileName(path: string): string {
  return path.split(/[/\\]/).pop() ?? '';
}

/** The extension of a file name, lower case and without the dot; `null` without one (a leading dot is no extension). */
function extensionOf(filename: string): string | null {
  const dot = filename.lastIndexOf('.');
  return dot > 0 ? filename.slice(dot + 1).toLowerCase() : null;
}

/** A document of the lists from a row of `journals.documents` or `accounts.documents`. */
export function documentOf(row: DocumentRow): Document {
  const path = row.path ?? '';
  const filename = fileName(path);
  return {
    datetime: `${row.date ?? ''}T${row.time ?? '00:00:00'}`,
    filename,
    path,
    extension: extensionOf(filename),
    account: row.account,
    trx_id: row.transaction_id,
  };
}

/**
 * The image formats a browser shows in an `<img>` loaded from the download endpoint, by file extension. The endpoint
 * sends a document's bytes without a content type, which browsers sniff for these raster formats. An SVG is shown only
 * with its content type, and TIFF or HEIC not by every browser: such a document opens in a new tab, like any other file.
 */
const PREVIEWABLE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp']);

/** The type of a document as the documents lists show it: the extension of its file name, upper case (`PDF`). */
export function documentType(document: Pick<Document, 'extension'>): string {
  return (document.extension ?? '').toUpperCase();
}

/** Whether a document of the lists can be previewed as an image: by the extension of its file name. */
export function canPreview(document: Pick<Document, 'extension'>): boolean {
  return document.extension != null && PREVIEWABLE_EXTENSIONS.has(document.extension);
}

/**
 * Whether the document at `path` can be previewed as an image: the `document` metadata of a transaction in the journal
 * preview, for the formats of {@link canPreview}.
 */
export function canPreviewPath(path: string): boolean {
  return canPreview({ extension: extensionOf(fileName(path)) });
}
