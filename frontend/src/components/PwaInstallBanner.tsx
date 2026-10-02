import { CircleCheck, Download, Smartphone } from 'lucide-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from './ui/button';
import { Item, ItemActions, ItemContent, ItemDescription, ItemMedia, ItemTitle } from './ui/item';

interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

function isStandalone() {
  return (
    window.matchMedia('(display-mode: standalone)').matches ||
    (window.navigator as Navigator & { standalone?: boolean }).standalone === true ||
    document.referrer.includes('android-app://')
  );
}

/**
 * Inline (not fixed) PWA install card: "installed" when running standalone, an Install button when the browser offers
 * `beforeinstallprompt`, otherwise manual "Add to Home Screen" instructions.
 */
const PwaInstallBanner: React.FC<{ className?: string }> = ({ className }) => {
  const { t } = useTranslation();
  const [installPrompt, setInstallPrompt] = useState<BeforeInstallPromptEvent | null>(null);
  const [isInstalled, setIsInstalled] = useState(false);

  useEffect(() => {
    setIsInstalled(isStandalone());
    const handleBeforeInstallPrompt = (e: Event) => {
      e.preventDefault();
      setInstallPrompt(e as BeforeInstallPromptEvent);
    };
    const handleInstalled = () => setIsInstalled(true);
    window.addEventListener('beforeinstallprompt', handleBeforeInstallPrompt);
    window.addEventListener('appinstalled', handleInstalled);
    return () => {
      window.removeEventListener('beforeinstallprompt', handleBeforeInstallPrompt);
      window.removeEventListener('appinstalled', handleInstalled);
    };
  }, []);

  const handleInstallClick = async () => {
    if (!installPrompt) return;
    await installPrompt.prompt();
    const choiceResult = await installPrompt.userChoice;
    if (choiceResult.outcome === 'accepted') setInstallPrompt(null);
  };

  if (isInstalled) {
    return (
      <Item variant="muted" className={className}>
        <ItemMedia variant="icon" className="size-9 rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
          <CircleCheck />
        </ItemMedia>
        <ItemContent>
          <ItemTitle>{t('PWA_INSTALL_BANNER.INSTALLED_TITLE')}</ItemTitle>
          <ItemDescription>{t('PWA_INSTALL_BANNER.INSTALLED_DESCRIPTION')}</ItemDescription>
        </ItemContent>
      </Item>
    );
  }

  return (
    <Item variant="outline" className={className}>
      <ItemMedia variant="icon" className="size-9 rounded-lg bg-primary/10 text-link dark:bg-primary/20 dark:text-primary-foreground">
        <Smartphone />
      </ItemMedia>
      <ItemContent className="min-w-0 basis-56">
        <ItemTitle>{t('PWA_INSTALL_BANNER.TITLE')}</ItemTitle>
        <ItemDescription className="line-clamp-none">
          {installPrompt ? t('PWA_INSTALL_BANNER.DESCRIPTION') : t('PWA_INSTALL_BANNER.MANUAL_DESCRIPTION')}
        </ItemDescription>
      </ItemContent>
      {installPrompt && (
        <ItemActions className="w-full sm:w-auto">
          <Button onClick={handleInstallClick} className="h-10 w-full sm:w-auto md:h-8">
            <Download />
            {t('PWA_INSTALL_BANNER.INSTALL_BUTTON')}
          </Button>
        </ItemActions>
      )}
    </Item>
  );
};

export default PwaInstallBanner;
