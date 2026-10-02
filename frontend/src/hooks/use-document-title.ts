import { useEffect } from 'react';

/** Sets `document.title` whenever the (non-empty) title changes. */
export function useDocumentTitle(title: string) {
  useEffect(() => {
    if (typeof title === 'string' && title.trim().length > 0) {
      document.title = title.trim();
    }
  }, [title]);
}
