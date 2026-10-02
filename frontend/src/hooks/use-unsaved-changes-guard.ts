import { useEffect } from 'react';

/**
 * While `dirty`, asks before the user leaves with unsaved changes:
 * - closing / reloading the tab (`beforeunload`);
 * - following an in-app link (sidebar, tab bar, "More" sheet, breadcrumb, …): a capture-phase click listener on `a[href]`
 *   runs before React Router's `<Link>` handler and cancels the click unless `window.confirm(message)` is accepted.
 *
 * The app uses `<BrowserRouter>`, which has no navigation blocker (`useBlocker` needs a data router), so the browser Back
 * button and programmatic `navigate()` calls are not covered.
 */
export function useUnsavedChangesGuard(dirty: boolean, message: string) {
  useEffect(() => {
    if (!dirty) return;

    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };

    const onClick = (event: MouseEvent) => {
      // New tab / window / download keeps this page (and its buffer) alive.
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      const anchor = event.target instanceof Element ? event.target.closest('a[href]') : null;
      if (!(anchor instanceof HTMLAnchorElement)) return;
      if ((anchor.target && anchor.target !== '_self') || anchor.hasAttribute('download')) return;
      const url = new URL(anchor.href, window.location.href);
      // Other origins unload the page, which `beforeunload` already covers.
      if (url.origin !== window.location.origin) return;
      if (url.pathname === window.location.pathname && url.search === window.location.search) return;
      if (window.confirm(message)) return;
      event.preventDefault();
      event.stopPropagation();
    };

    window.addEventListener('beforeunload', onBeforeUnload);
    document.addEventListener('click', onClick, true);
    return () => {
      window.removeEventListener('beforeunload', onBeforeUnload);
      document.removeEventListener('click', onClick, true);
    };
  }, [dirty, message]);
}
