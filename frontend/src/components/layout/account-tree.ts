import { writeLocalStorage } from '@/hooks/use-local-storage';

export const accountCollapseKey = (path: string) => `account-collapse-${path}`;

/** Store the expanded state of many tree nodes at once and notify every mounted `AccountLine`. */
export function setAccountsExpanded(paths: string[], expanded: boolean) {
  paths.forEach((path) => writeLocalStorage(accountCollapseKey(path), expanded));
}
