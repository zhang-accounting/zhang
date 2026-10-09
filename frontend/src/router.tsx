import { useAtomValue, useSetAtom } from 'jotai';
import { lazy, type ReactNode, Suspense, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { matchRoutes, Route, Routes, useLocation } from 'react-router';
import { PageShell } from './components/layout';
import { Skeleton } from './components/ui/skeleton';
import { useDocumentTitle } from './hooks/use-document-title';
import * as NAV from './layout/nav-links';
import { breadcrumbAtom, titleAtom } from './states/basic';

// Every page is a chunk of its own, loaded on its first visit, so the first load ships the shell and one page; the chart and
// editor libraries are chunks of their own (vite.config.ts), shared by the pages that draw charts or edit text.
const Home = lazy(() => import('./pages/Home'));
const Journals = lazy(() => import('./pages/Journals'));
const Accounts = lazy(() => import('./pages/Accounts'));
const SingleAccount = lazy(() => import('./pages/SingleAccount'));
const Commodities = lazy(() => import('./pages/Commodities'));
const SingleCommodity = lazy(() => import('./pages/SingleCommodity'));
const Documents = lazy(() => import('./pages/Documents'));
const Budgets = lazy(() => import('./pages/Budgets'));
const SingleBudget = lazy(() => import('./pages/SingleBudget'));
const RawEdit = lazy(() => import('./pages/RawEdit'));
const Report = lazy(() => import('./pages/Report'));
const ToolList = lazy(() => import('./pages/tools/ToolList'));
const BatchBalance = lazy(() => import('./pages/tools/BatchBalance'));
const Settings = lazy(() => import('./pages/Settings'));
const Explore = lazy(() => import('./pages/Explore'));

/**
 * A page and what the shell shows for it. The breadcrumb is the page's `section`, then either the `:param` the page shows
 * (an account, a commodity, a budget) or its own `label` (an i18n key); the document title is the trail read leaf-first.
 */
interface PageRoute {
  path: string;
  element: ReactNode;
  section: NAV.NavLink;
  param?: string;
  label?: string;
  /** The section crumb keeps the page's query string, so "Budgets" returns to the month on screen. */
  keepSearch?: boolean;
  /** The page sets `document.title` itself (the raw editor adds the open file). */
  ownTitle?: boolean;
}

const ROUTES: PageRoute[] = [
  { path: '/', element: <Home />, section: NAV.DASHBOARD_LINK },
  { path: '/journals', element: <Journals />, section: NAV.JOURNALS_LINK },
  { path: '/accounts', element: <Accounts />, section: NAV.ACCOUNTS_LINK },
  { path: '/accounts/:accountName', element: <SingleAccount />, section: NAV.ACCOUNTS_LINK, param: 'accountName' },
  { path: '/commodities', element: <Commodities />, section: NAV.COMMODITIES_LINK },
  { path: '/commodities/:commodityName', element: <SingleCommodity />, section: NAV.COMMODITIES_LINK, param: 'commodityName' },
  { path: '/documents', element: <Documents />, section: NAV.DOCUMENTS_LINK },
  // The month lives in the page's own switcher; a "Budgets > Oct 2026" crumb would make the mobile back button point at this page.
  { path: '/budgets', element: <Budgets />, section: NAV.BUDGETS_LINK },
  { path: '/budgets/:budgetName', element: <SingleBudget />, section: NAV.BUDGETS_LINK, param: 'budgetName', keepSearch: true },
  { path: '/edit', element: <RawEdit />, section: NAV.RAW_EDITING_LINK, ownTitle: true },
  { path: '/report', element: <Report />, section: NAV.REPORT_LINK },
  { path: '/tools', element: <ToolList />, section: NAV.TOOLS_LINK },
  { path: '/tools/batch-balance', element: <BatchBalance />, section: NAV.TOOLS_LINK, label: 'tools.batch_balance_title' },
  { path: '/settings', element: <Settings />, section: NAV.SETTINGS_LINK },
  { path: '/explore', element: <Explore />, section: NAV.QUERY_LINK },
];

/** Sets the breadcrumb (`breadcrumbAtom`) and the document title of the page on screen, from its entry in `ROUTES`. */
function PageMeta() {
  const { t } = useTranslation();
  const location = useLocation();
  const ledgerTitle = useAtomValue(titleAtom);
  const setBreadcrumb = useSetAtom(breadcrumbAtom);
  const match = matchRoutes(ROUTES, location)?.[0];
  const route = match?.route as PageRoute | undefined;
  const section = route && { label: route.section.label, uri: route.section.uri + (route.keepSearch ? location.search : '') };
  const leaf = route?.param ? { label: match?.params[route.param] ?? '', noTranslate: true } : route?.label ? { label: route.label } : undefined;
  const crumbs = section ? (leaf ? [section, { ...leaf, uri: match?.pathname ?? '' }] : [section]) : [];
  const key = JSON.stringify(crumbs);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- `key` is the crumbs' content; a new array alone is no new trail
  useEffect(() => setBreadcrumb(crumbs), [key, setBreadcrumb]);
  const labels = crumbs.map((crumb) => ('noTranslate' in crumb ? crumb.label : t(crumb.label))).reverse();
  // an empty title leaves `document.title` alone: no page matched, or the page owns its title
  useDocumentTitle(route && !route.ownTitle ? `${labels.join(' | ')} - ${ledgerTitle}` : '');
  return null;
}

/** The page's column while its chunk loads: the outline of a page header, in the style of the pages' own skeletons. */
function PageLoading() {
  return (
    <PageShell aria-busy>
      <div className="flex flex-col gap-3">
        <Skeleton className="h-7 w-40" />
        <Skeleton className="h-4 w-72 max-w-full" />
      </div>
    </PageShell>
  );
}

export function Router() {
  return (
    <>
      <PageMeta />
      <Suspense fallback={<PageLoading />}>
        <Routes>
          {ROUTES.map((route) => (
            <Route key={route.path} path={route.path} element={route.element} />
          ))}
        </Routes>
      </Suspense>
    </>
  );
}
