import type { LucideIcon } from 'lucide-react';
import { ChartLine, Coins, FilePenLine, FileStack, LayoutDashboard, NotebookText, Scale } from 'lucide-react';
import { PiggyBank, SearchCode, Settings, WalletMinimal, Wrench } from 'lucide-react';

/** A navigable top-level route. `label` is an i18n key; `shortLabel` (optional) is used by the mobile tab bar. */
export interface NavLink {
  icon: LucideIcon;
  label: string;
  shortLabel?: string;
  uri: string;
}

export const DASHBOARD_LINK: NavLink = { icon: LayoutDashboard, label: 'NAV_DASHBOARD', uri: '/' };
export const JOURNALS_LINK: NavLink = { icon: NotebookText, label: 'NAV_JOURNALS', uri: '/journals' };
export const ACCOUNTS_LINK: NavLink = { icon: WalletMinimal, label: 'NAV_ACCOUNTS', uri: '/accounts' };
/** Same route as `ACCOUNTS_LINK`, labelled "Balance sheet" in the desktop sidebar. */
export const BALANCE_SHEET_LINK: NavLink = { icon: Scale, label: 'NAV_BALANCE_SHEET', uri: '/accounts' };
export const COMMODITIES_LINK: NavLink = { icon: Coins, label: 'NAV_COMMODITIES', uri: '/commodities' };
export const BUDGETS_LINK: NavLink = { icon: PiggyBank, label: 'NAV_BUDGETS', uri: '/budgets' };
export const DOCUMENTS_LINK: NavLink = { icon: FileStack, label: 'NAV_DOCUMENTS', uri: '/documents' };
export const REPORT_LINK: NavLink = { icon: ChartLine, label: 'NAV_REPORT', shortLabel: 'NAV_REPORT_SHORT', uri: '/report' };
export const RAW_EDITING_LINK: NavLink = { icon: FilePenLine, label: 'NAV_RAW_EDITING', uri: '/edit' };
export const TOOLS_LINK: NavLink = { icon: Wrench, label: 'NAV_TOOLS', uri: '/tools' };
export const QUERY_LINK: NavLink = { icon: SearchCode, label: 'NAV_QUERY', uri: '/explore' };
export const SETTINGS_LINK: NavLink = { icon: Settings, label: 'NAV_SETTING', uri: '/settings' };

/** Desktop sidebar: primary items, the collapsible "More" group and the footer items. */
export const SIDEBAR_PRIMARY_LINKS: NavLink[] = [DASHBOARD_LINK, JOURNALS_LINK, REPORT_LINK, BALANCE_SHEET_LINK, BUDGETS_LINK];
export const SIDEBAR_MORE_LINKS: NavLink[] = [COMMODITIES_LINK, DOCUMENTS_LINK, RAW_EDITING_LINK, QUERY_LINK];
export const SIDEBAR_FOOTER_LINKS: NavLink[] = [TOOLS_LINK, SETTINGS_LINK];

/** Bottom tab bar (mobile); everything else lives in the "More" sheet. */
export const MOBILE_PRIMARY_LINKS: NavLink[] = [DASHBOARD_LINK, JOURNALS_LINK, ACCOUNTS_LINK, REPORT_LINK];
export const MOBILE_MORE_LINKS: NavLink[] = [BUDGETS_LINK, COMMODITIES_LINK, DOCUMENTS_LINK, RAW_EDITING_LINK, TOOLS_LINK, QUERY_LINK, SETTINGS_LINK];

export const UPGRADE_GUIDE_URL = 'https://zhang-accounting.kilerd.me/deployment/upgrading/';
export const BUDGET_DOCS_URL = 'https://zhang-accounting.kilerd.me/reference/directives/budget/';

/** `/` matches exactly; other links also match their sub-routes (e.g. `/accounts/Assets:Bank`). */
export function isLinkActive(pathname: string, uri: string) {
  if (uri === '/') return pathname === '/';
  return pathname === uri || pathname.startsWith(`${uri}/`);
}
