import React from 'react';
import { WifiOff } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNetworkState } from 'react-use';
import { cn } from '@/lib/utils';

/** Fixed banner shown while the browser is offline; sits above the mobile tab bar. */
const NetworkStatus: React.FC = () => {
  const { t } = useTranslation();
  const { online: isOnline = true } = useNetworkState();

  if (isOnline) return null;

  return (
    <div
      role="alert"
      className={cn(
        'fixed inset-x-0 bottom-[calc(4rem+env(safe-area-inset-bottom))] z-50 md:bottom-0',
        'flex items-center justify-center gap-2 bg-destructive px-4 py-2.5 text-center text-sm font-medium text-background',
      )}
    >
      <WifiOff className="size-4 shrink-0" />
      {t('SHELL_NETWORK_OFFLINE_BANNER')}
    </div>
  );
};

export default NetworkStatus;
