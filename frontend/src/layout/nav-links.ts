import type { LucideIcon } from 'lucide-react';
import { ChartArea, Coins, FilePenLine, FileStack, LayoutDashboard, NotebookText } from 'lucide-react';
import { PiggyBank, SearchCode, Settings, WalletMinimal, Wrench } from 'lucide-react';

/** A navigable top-level route. `label` is an i18n key. Pages also pass these to `breadcrumbAtom`. */
export interface NavLink {
  icon: LucideIcon;
  label: string;
  uri: string;
}

export const DASHBOARD_LINK: NavLink = { icon: LayoutDashboard, label: 'NAV_DASHBOARD', uri: '/' };
export const JOURNALS_LINK: NavLink = { icon: NotebookText, label: 'NAV_JOURNALS', uri: '/journals' };
export const ACCOUNTS_LINK: NavLink = { icon: WalletMinimal, label: 'NAV_ACCOUNTS', uri: '/accounts' };
export const COMMODITIES_LINK: NavLink = { icon: Coins, label: 'NAV_COMMODITIES', uri: '/commodities' };
export const BUDGETS_LINK: NavLink = { icon: PiggyBank, label: 'NAV_BUDGETS', uri: '/budgets' };
export const DOCUMENTS_LINK: NavLink = { icon: FileStack, label: 'NAV_DOCUMENTS', uri: '/documents' };
export const REPORT_LINK: NavLink = { icon: ChartArea, label: 'NAV_REPORT', uri: '/report' };
export const RAW_EDITING_LINK: NavLink = { icon: FilePenLine, label: 'NAV_RAW_EDITING', uri: '/edit' };
export const TOOLS_LINK: NavLink = { icon: Wrench, label: 'NAV_TOOLS', uri: '/tools' };
export const QUERY_LINK: NavLink = { icon: SearchCode, label: 'NAV_QUERY', uri: '/explore' };
export const SETTINGS_LINK: NavLink = { icon: Settings, label: 'NAV_SETTING', uri: '/settings' };

/** Sidebar sections (desktop). `label` is an i18n key. */
export const NAV_GROUPS: { label: string; links: NavLink[] }[] = [
  { label: 'NAV_GROUP_OVERVIEW', links: [DASHBOARD_LINK, JOURNALS_LINK, ACCOUNTS_LINK, REPORT_LINK] },
  { label: 'NAV_GROUP_LEDGER', links: [COMMODITIES_LINK, BUDGETS_LINK, DOCUMENTS_LINK] },
  { label: 'NAV_GROUP_TOOLS', links: [RAW_EDITING_LINK, TOOLS_LINK, QUERY_LINK] },
];

/** Bottom tab bar (mobile); everything else lives in the "More" sheet. */
export const MOBILE_PRIMARY_LINKS: NavLink[] = [DASHBOARD_LINK, JOURNALS_LINK, ACCOUNTS_LINK, REPORT_LINK];
export const MOBILE_MORE_LINKS: NavLink[] = [
  ...NAV_GROUPS.flatMap((group) => group.links).filter((link) => !MOBILE_PRIMARY_LINKS.includes(link)),
  SETTINGS_LINK,
];

export const UPGRADE_GUIDE_URL = 'https://zhang-accounting.kilerd.me/installation/4-upgrade/';

/** `/` matches exactly; other links also match their sub-routes (e.g. `/accounts/Assets:Bank`). */
export function isLinkActive(pathname: string, uri: string) {
  if (uri === '/') return pathname === '/';
  return pathname === uri || pathname.startsWith(`${uri}/`);
}
