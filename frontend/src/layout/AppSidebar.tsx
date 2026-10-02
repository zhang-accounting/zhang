import { useAtomValue } from 'jotai';
import { ArrowUpRight, RotateCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Link, useLocation } from 'react-router-dom';
import { cn } from '@/lib/utils';
import { buttonVariants } from '@/components/ui/button';
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from '@/components/ui/sidebar';
import { errorCountAtom } from '@/states/errors';
import { titleAtom, updatableVersionAtom, versionAtom } from '@/states/basic';
import { DASHBOARD_LINK, isLinkActive, NAV_GROUPS, NavLink, SETTINGS_LINK, UPGRADE_GUIDE_URL } from './nav-links';
import { useReloadLedger } from './use-reload-ledger';

const UPGRADE_BUTTON_CLASS = cn(buttonVariants({ size: 'sm' }), 'mt-2 w-full');
const BADGE_CLASS = 'bg-destructive/10 text-destructive peer-data-active/menu-button:text-destructive';

function NavItem({ link, badge }: { link: NavLink; badge?: number }) {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const label = t(link.label);
  return (
    <SidebarMenuItem>
      <SidebarMenuButton render={<Link to={link.uri} />} isActive={isLinkActive(pathname, link.uri)} tooltip={label}>
        <link.icon />
        <span>{label}</span>
      </SidebarMenuButton>
      {!!badge && <SidebarMenuBadge className={BADGE_CLASS}>{badge}</SidebarMenuBadge>}
    </SidebarMenuItem>
  );
}

/** Desktop (>= md) navigation: ledger header, grouped links, settings + update notice in the footer. Collapses to icons. */
export function AppSidebar() {
  const { t } = useTranslation();
  const errorsCount = useAtomValue(errorCountAtom);
  const ledgerTitle = useAtomValue(titleAtom);
  const version = useAtomValue(versionAtom);
  const updatableVersion = useAtomValue(updatableVersionAtom);
  const reloadLedger = useReloadLedger();

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" render={<Link to="/" />} tooltip={ledgerTitle ?? undefined}>
              {/* Decorative: the ledger title next to it names the link. Stays visible when the sidebar collapses to icons. */}
              <img src="/otter-192.png" alt="" className="size-8 shrink-0 rounded-lg" />
              <span className="grid flex-1 text-left leading-tight">
                <span className="truncate font-semibold">{ledgerTitle}</span>
                <span className="truncate text-xs text-muted-foreground">Zhang {version ?? ''}</span>
              </span>
            </SidebarMenuButton>
            <SidebarMenuAction onClick={reloadLedger} title={t('SHELL_RELOAD_LEDGER')} aria-label={t('SHELL_RELOAD_LEDGER')}>
              <RotateCw />
            </SidebarMenuAction>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent>
        {NAV_GROUPS.map((group) => (
          <SidebarGroup key={group.label}>
            <SidebarGroupLabel>{t(group.label)}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {group.links.map((link) => (
                  <NavItem key={link.uri} link={link} badge={link === DASHBOARD_LINK ? errorsCount : undefined} />
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
      </SidebarContent>
      <SidebarFooter>
        {updatableVersion && (
          <div className="rounded-lg border bg-background p-3 text-sm group-data-[collapsible=icon]:hidden">
            <p className="font-medium">{t('SHELL_UPDATE_AVAILABLE', { version: updatableVersion })}</p>
            <a href={UPGRADE_GUIDE_URL} target="_blank" rel="noreferrer" className={UPGRADE_BUTTON_CLASS}>
              {t('SHELL_UPDATE_GUIDE')}
              <ArrowUpRight />
            </a>
          </div>
        )}
        <SidebarMenu>
          <NavItem link={SETTINGS_LINK} />
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}
