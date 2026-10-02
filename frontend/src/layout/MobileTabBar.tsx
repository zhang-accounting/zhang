import { useAtomValue } from 'jotai';
import { ArrowUpRight, Ellipsis, RotateCw } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, useLocation } from 'react-router-dom';
import { Button, buttonVariants } from '@/components/ui/button';
import { SheetCloseButton } from '@/components/layout/SheetCloseButton';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, SheetTrigger } from '@/components/ui/sheet';
import { useLanguage } from '@/hooks/use-language';
import { LANGUAGES } from '@/lib/languages';
import { cn } from '@/lib/utils';
import { titleAtom, updatableVersionAtom, versionAtom } from '@/states/basic';
import { errorCountAtom } from '@/states/errors';
import { DASHBOARD_LINK, isLinkActive, MOBILE_MORE_LINKS, MOBILE_PRIMARY_LINKS, UPGRADE_GUIDE_URL } from './nav-links';
import { OnlineStatus } from './OnlineStatus';
import { THEMES } from './themes';
import { useReloadLedger } from './use-reload-ledger';

const TAB_CLASS = cn(
  'relative flex h-full w-full flex-col items-center justify-center gap-1',
  'text-[11px] font-medium text-muted-foreground outline-none transition-colors active:bg-muted',
  'focus-visible:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
);
const TILE_CLASS = cn(
  'flex h-18 flex-col items-center justify-center gap-1.5 rounded-xl border bg-card px-1',
  'text-center text-xs font-medium transition-colors outline-none active:bg-muted focus-visible:ring-2 focus-visible:ring-ring',
);
const BAR_CLASS = cn(
  'fixed inset-x-0 bottom-0 z-40 border-t bg-card/95 pb-[env(safe-area-inset-bottom)] md:hidden',
  'backdrop-blur supports-backdrop-filter:bg-card/80',
);
const UPGRADE_BUTTON_CLASS = cn(buttonVariants({ variant: 'outline' }), 'h-10');
const BADGE_CLASS = 'absolute -top-1 -right-2 min-w-4 rounded-full bg-destructive px-1 text-[10px] leading-4 text-background';

function SegmentedButton({ active, className, ...props }: React.ComponentProps<typeof Button> & { active: boolean }) {
  return (
    <Button
      variant={active ? 'secondary' : 'ghost'}
      aria-pressed={active}
      className={cn('h-10 flex-1', active && 'ring-1 ring-border', className)}
      {...props}
    />
  );
}

/** "More" bottom sheet: secondary routes, theme, language, reload, update notice. */
function MoreSheet() {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const [open, setOpen] = useState(false);
  const ledgerTitle = useAtomValue(titleAtom);
  const version = useAtomValue(versionAtom);
  const updatableVersion = useAtomValue(updatableVersionAtom);
  const { theme, setTheme } = useTheme();
  const [lang, setLang] = useLanguage();
  const reloadLedger = useReloadLedger();
  const moreActive = MOBILE_MORE_LINKS.some((link) => isLinkActive(pathname, link.uri));

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger render={<button type="button" className={cn(TAB_CLASS, moreActive && 'text-link')} />}>
        <Ellipsis className="size-5" />
        <span>{t('NAV_MORE')}</span>
      </SheetTrigger>
      <SheetContent
        side="bottom"
        showCloseButton={false}
        className="max-h-[85svh] gap-0 overflow-y-auto overscroll-contain rounded-t-2xl pb-[env(safe-area-inset-bottom)]"
      >
        <SheetHeader className="pr-12">
          <SheetTitle className="flex min-w-0 items-center gap-2">
            <img src="/otter-192.png" alt="" className="size-7 shrink-0 rounded-md" />
            <span className="truncate">{ledgerTitle}</span>
          </SheetTitle>
          <SheetDescription className="flex items-center gap-2">
            <span>Zhang {version ?? ''}</span>
            <OnlineStatus showLabel className="h-6 px-0" />
          </SheetDescription>
        </SheetHeader>

        <nav className="grid grid-cols-3 gap-2 px-4" aria-label={t('NAV_MORE')}>
          {MOBILE_MORE_LINKS.map((link) => {
            const active = isLinkActive(pathname, link.uri);
            return (
              <Link
                key={link.uri}
                to={link.uri}
                onClick={() => setOpen(false)}
                aria-current={active ? 'page' : undefined}
                className={cn(TILE_CLASS, active && 'border-link/40 bg-link/5 text-link')}
              >
                <link.icon className="size-5" />
                <span className="line-clamp-2">{t(link.label)}</span>
              </Link>
            );
          })}
        </nav>

        <div className="flex flex-col gap-4 p-4">
          <div className="flex flex-col gap-2">
            <span className="text-xs font-medium text-muted-foreground">{t('SHELL_THEME')}</span>
            <div className="flex gap-1 rounded-lg bg-muted p-1">
              {THEMES.map((item) => (
                <SegmentedButton key={item.value} active={(theme ?? 'system') === item.value} onClick={() => setTheme(item.value)}>
                  <item.icon />
                  {t(item.label)}
                </SegmentedButton>
              ))}
            </div>
          </div>
          <div className="flex flex-col gap-2">
            <span className="text-xs font-medium text-muted-foreground">{t('SHELL_LANGUAGE')}</span>
            <div className="flex gap-1 rounded-lg bg-muted p-1">
              {LANGUAGES.map((language) => (
                <SegmentedButton key={language.value} active={lang === language.value} onClick={() => setLang(language.value)}>
                  {language.label}
                </SegmentedButton>
              ))}
            </div>
          </div>
          {updatableVersion && (
            <a href={UPGRADE_GUIDE_URL} target="_blank" rel="noreferrer" className={UPGRADE_BUTTON_CLASS}>
              {t('SHELL_UPDATE_AVAILABLE', { version: updatableVersion })}
              <ArrowUpRight />
            </a>
          )}
          <Button variant="outline" className="h-10" onClick={reloadLedger}>
            <RotateCw />
            {t('SHELL_RELOAD_LEDGER')}
          </Button>
        </div>
        <SheetCloseButton />
      </SheetContent>
    </Sheet>
  );
}

/** Mobile (< md) bottom tab bar: 4 primary routes + "More". Fixed, safe-area aware, 64px tall tabs. */
export function MobileTabBar() {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const errorsCount = useAtomValue(errorCountAtom);

  return (
    <nav className={BAR_CLASS} aria-label={t('SHELL_PRIMARY_NAV')}>
      <ul className="grid h-16 grid-cols-5">
        {MOBILE_PRIMARY_LINKS.map((link) => {
          const active = isLinkActive(pathname, link.uri);
          return (
            <li key={link.uri}>
              <Link to={link.uri} aria-current={active ? 'page' : undefined} className={cn(TAB_CLASS, active && 'text-link')}>
                <span className="relative">
                  <link.icon className="size-5" />
                  {link === DASHBOARD_LINK && errorsCount > 0 && <span className={BADGE_CLASS}>{errorsCount}</span>}
                </span>
                <span className="max-w-full truncate px-1">{t(link.shortLabel ?? link.label)}</span>
              </Link>
            </li>
          );
        })}
        <li>
          <MoreSheet />
        </li>
      </ul>
    </nav>
  );
}
