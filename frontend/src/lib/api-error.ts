import { ApiError } from 'openapi-typescript-fetch';
import { reportUnauthorized } from '@/api/fetcher';

/** Server error bodies are `{ message }` (zhang-server `error.rs`); long HTML / text bodies are cut for toasts. */
const MAX_LENGTH = 300;

function clip(text: string) {
  const trimmed = text.trim();
  return trimmed.length > MAX_LENGTH ? `${trimmed.slice(0, MAX_LENGTH)}…` : trimmed;
}

function messageFromBody(body: unknown): string | undefined {
  if (typeof body === 'string') return clip(body) || undefined;
  if (body && typeof body === 'object' && 'message' in body) {
    const message = (body as { message: unknown }).message;
    if (typeof message === 'string' && message.trim() !== '') return clip(message);
  }
  return undefined;
}

function statusLine(status: number, statusText: string) {
  return `${status} ${statusText}`.trim();
}

async function readBody(response: Response): Promise<unknown> {
  try {
    const text = await response.text();
    try {
      return JSON.parse(text);
    } catch {
      return text;
    }
  } catch {
    return undefined;
  }
}

/**
 * Error carrying the server message of a failed plain `fetch` (multipart uploads bypass the generated client). A 401 outside
 * `/api/auth/*` also ends the session (login page), like the generated client's middleware.
 */
export async function responseError(response: Response): Promise<Error> {
  reportUnauthorized(response.url, response.status);
  return new Error(messageFromBody(await readBody(response)) ?? statusLine(response.status, response.statusText));
}

/**
 * Human-readable description of a failed request for toasts: the server's `{ message }` when there is one, otherwise the
 * HTTP status / error text. Handles the generated client's `ApiError`, a raw `Response`, `Error`s and strings.
 */
export async function apiErrorMessage(error: unknown): Promise<string> {
  if (error instanceof ApiError) return messageFromBody(error.data) ?? statusLine(error.status, error.statusText);
  if (error instanceof Response) return messageFromBody(await readBody(error)) ?? statusLine(error.status, error.statusText);
  if (error instanceof Error) return error.message || String(error);
  return messageFromBody(error) ?? String(error);
}
