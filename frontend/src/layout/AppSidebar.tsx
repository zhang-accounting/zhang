import { useAtomValue } from 'jotai';
import { ArrowUpRight, ChevronRight, LogOut, RotateCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link, useLocation } from 'react-router';
import NewTransactionButton from '@/components/NewTransactionButton';
import { buttonVariants } from '@/components/ui/button';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible';
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
  SidebarTrigger,
} from '@/components/ui/sidebar';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { useLocalStorage } from '@/hooks/use-local-storage';
import { cn } from '@/lib/utils';
import { canSignOutAtom } from '@/states/auth';
import { titleAtom, updatableVersionAtom, versionAtom } from '@/states/basic';
import { errorCountAtom } from '@/states/errors';
import { LanguageSwitch } from './LanguageSwitch';
import { DASHBOARD_LINK, isLinkActive, NavLink, SIDEBAR_FOOTER_LINKS, SIDEBAR_MORE_LINKS, SIDEBAR_PRIMARY_LINKS, UPGRADE_GUIDE_URL } from './nav-links';
import { OnlineStatus } from './OnlineStatus';
import { SidebarAccounts } from './SidebarAccounts';
import { ThemeToggle } from './ThemeToggle';
import { useReloadLedger } from './use-reload-ledger';
import { useSignOut } from './use-sign-out';

const UPGRADE_BUTTON_CLASS = cn(buttonVariants({ size: 'sm' }), 'mt-2 w-full');
const BADGE_CLASS = 'bg-destructive/10 text-destructive peer-data-active/menu-button:text-destructive';

/**
 * Nav item (34px): foreground-2 label with a muted icon; hover = neutral `accent`; active = brand tint (`sidebar-accent`),
 * `sidebar-accent-foreground` label, medium weight and a `link` icon (the turquoise primary is a fill colour only).
 */
const NAV_BUTTON_CLASS = cn(
  'h-8.5 gap-2.5 px-2.5 text-foreground-2 [&_svg]:text-muted-foreground',
  'hover:bg-accent hover:text-foreground active:bg-accent active:text-foreground',
  'data-active:bg-sidebar-accent data-active:text-sidebar-accent-foreground data-active:hover:bg-sidebar-accent data-active:[&_svg]:text-link',
);
/** Muted 28px icon buttons (reload, footer tools). */
const ICON_BUTTON_CLASS = 'size-7 text-muted-foreground hover:bg-accent hover:text-foreground';

function NavItem({ link, badge }: { link: NavLink; badge?: number }) {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const label = t(link.label);
  return (
    <SidebarMenuItem>
      <SidebarMenuButton render={<Link to={link.uri} />} isActive={isLinkActive(pathname, link.uri)} tooltip={label} className={NAV_BUTTON_CLASS}>
        <link.icon />
        <span>{label}</span>
      </SidebarMenuButton>
      {!!badge && <SidebarMenuBadge className={BADGE_CLASS}>{badge}</SidebarMenuBadge>}
    </SidebarMenuItem>
  );
}

/** Collapsible "More" group (collapsed by default, remembered in localStorage); it opens itself while one of its routes is active. */
function MoreGroup() {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const [open, setOpen] = useLocalStorage({ key: 'sidebar-more-open', defaultValue: false });
  const containsActive = SIDEBAR_MORE_LINKS.some((link) => isLinkActive(pathname, link.uri));
  const expanded = open || containsActive;

  return (
    <Collapsible open={expanded} onOpenChange={(next) => setOpen(next)} render={<SidebarMenuItem />}>
      <CollapsibleTrigger render={<SidebarMenuButton tooltip={t('NAV_MORE')} className={cn(NAV_BUTTON_CLASS, 'group/more')} />} aria-label={t('NAV_MORE')}>
        <ChevronRight className="transition-transform group-data-panel-open/more:rotate-90 motion-reduce:transition-none" />
        <span>{t('NAV_MORE')}</span>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <SidebarMenu>
          {SIDEBAR_MORE_LINKS.map((link) => (
            <NavItem key={link.uri} link={link} />
          ))}
        </SidebarMenu>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Footer icon button ending the session (only when the server has auth on). */
function SignOutButton() {
  const { t } = useTranslation();
  const signOut = useSignOut();
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            onClick={() => void signOut()}
            aria-label={t('auth.sign_out')}
            className={cn(buttonVariants({ variant: 'ghost', size: 'icon' }), ICON_BUTTON_CLASS)}
          />
        }
      >
        <LogOut />
      </TooltipTrigger>
      <TooltipContent side="top">{t('auth.sign_out')}</TooltipContent>
    </Tooltip>
  );
}

/**
 * Desktop (>= md) navigation, modelled on 多少记账 + Actual Budget: ledger header, "New transaction", primary nav with a
 * collapsible "More" group, the accounts list with balances, and a footer with Tools / Settings plus theme, language, sign-out
 * (auth on) and collapse buttons. Collapses to icons (Ctrl/Cmd+B); the accounts list is hidden then.
 */
export function AppSidebar() {
  const { t } = useTranslation();
  const errorsCount = useAtomValue(errorCountAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  const version = useAtomValue(versionAtom);
  const updatableVersion = useAtomValue(updatableVersionAtom);
  const canSignOut = useAtomValue(canSignOutAtom);
  const reloadLedger = useReloadLedger();

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader className="gap-2.5 px-2.5 pt-3.5 pb-1 group-data-[collapsible=icon]:px-2">
        <div className="flex items-center gap-2.5 px-1.5 group-data-[collapsible=icon]:px-0">
          <Link to="/" className="shrink-0 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring" aria-label={ledgerTitle ?? undefined}>
            <img src="/otter-192.png" alt="" className="size-7 rounded-md group-data-[collapsible=icon]:size-8" />
          </Link>
          <div className="flex min-w-0 flex-1 flex-col group-data-[collapsible=icon]:hidden">
            <span className="truncate text-sm leading-tight font-semibold">{ledgerTitle}</span>
            <span className="flex min-w-0 items-center gap-1.5 text-xs leading-tight text-muted-foreground">
              <span className="shrink-0">Zhang {version ?? ''}</span>
              <OnlineStatus showLabel className="h-auto min-w-0 gap-1 px-0 [&>span:last-child]:truncate" />
            </span>
          </div>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  onClick={reloadLedger}
                  aria-label={t('SHELL_RELOAD_LEDGER')}
                  className={cn(buttonVariants({ variant: 'ghost', size: 'icon' }), ICON_BUTTON_CLASS, 'shrink-0 group-data-[collapsible=icon]:hidden')}
                />
              }
            >
              <RotateCw />
            </TooltipTrigger>
            <TooltipContent>{t('SHELL_RELOAD_LEDGER')}</TooltipContent>
          </Tooltip>
        </div>
        <NewTransactionButton variant="sidebar" />
      </SidebarHeader>

      <SidebarContent className="overflow-y-auto">
        <nav aria-label={t('SHELL_PRIMARY_NAV')} className="shrink-0 px-2.5 pt-1.5 group-data-[collapsible=icon]:px-2">
          <SidebarMenu className="gap-0.5">
            {SIDEBAR_PRIMARY_LINKS.map((link) => (
              <NavItem key={link.label} link={link} badge={link === DASHBOARD_LINK ? errorsCount : undefined} />
            ))}
            <MoreGroup />
          </SidebarMenu>
        </nav>
        <SidebarAccounts className="min-h-32 flex-1 group-data-[collapsible=icon]:hidden" />
      </SidebarContent>

      {updatableVersion && (
        <div className="mx-2.5 mb-2 rounded-lg border bg-card p-3 text-sm group-data-[collapsible=icon]:hidden">
          <p className="font-medium">{t('SHELL_UPDATE_AVAILABLE', { version: updatableVersion })}</p>
          <a href={UPGRADE_GUIDE_URL} target="_blank" rel="noreferrer" className={UPGRADE_BUTTON_CLASS}>
            {t('SHELL_UPDATE_GUIDE')}
            <ArrowUpRight />
          </a>
        </div>
      )}

      <SidebarFooter className="gap-1 border-t border-sidebar-border px-2.5 pt-2 pb-2.5 group-data-[collapsible=icon]:px-2">
        <SidebarMenu className="gap-0.5">
          {SIDEBAR_FOOTER_LINKS.map((link) => (
            <NavItem key={link.uri} link={link} />
          ))}
        </SidebarMenu>
        <div className="flex items-center gap-1 px-1 group-data-[collapsible=icon]:flex-col group-data-[collapsible=icon]:px-0">
          <ThemeToggle className={ICON_BUTTON_CLASS} side="top" />
          <LanguageSwitch className={ICON_BUTTON_CLASS} side="top" />
          {canSignOut && <SignOutButton />}
          <SidebarTrigger
            className={cn(ICON_BUTTON_CLASS, 'ml-auto group-data-[collapsible=icon]:ml-0')}
            aria-label={t('SHELL_TOGGLE_SIDEBAR')}
            title={`${t('SHELL_TOGGLE_SIDEBAR')} (⌘/Ctrl B)`}
          />
        </div>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}
