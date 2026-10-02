export const accountCollapseKey = (path: string) => `account-collapse-${path}`;

/** Store the expanded state of many tree nodes at once and notify every mounted `AccountLine`. */
export function setAccountsExpanded(paths: string[], expanded: boolean) {
  paths.forEach((path) => {
    const key = accountCollapseKey(path);
    try {
      window.localStorage.setItem(key, JSON.stringify(expanded));
    } catch {
      // storage unavailable: nothing to persist
    }
    window.dispatchEvent(new CustomEvent('zhang:local-storage', { detail: key }));
  });
}
