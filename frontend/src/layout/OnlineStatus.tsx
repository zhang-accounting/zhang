import { useAtomValue } from 'jotai';
import { useTranslation } from 'react-i18next';
import { useNetworkState } from 'react-use';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import { onlineAtom } from '@/states/basic';

type Status = 'connected' | 'disconnected' | 'offline';

const STATUS_LABEL: Record<Status, string> = {
  connected: 'SHELL_STATUS_CONNECTED',
  disconnected: 'SHELL_STATUS_DISCONNECTED',
  offline: 'SHELL_STATUS_OFFLINE',
};

const STATUS_DOT: Record<Status, string> = {
  connected: 'bg-emerald-500',
  disconnected: 'bg-amber-500',
  offline: 'bg-destructive',
};

const TRIGGER_CLASS = 'inline-flex h-8 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground';

/** Dot (+ optional label) for browser connectivity and the ledger server SSE connection (`onlineAtom`). */
export function OnlineStatus({ showLabel = false, className }: { showLabel?: boolean; className?: string }) {
  const { t } = useTranslation();
  const { online: browserOnline = true } = useNetworkState();
  const serverOnline = useAtomValue(onlineAtom);
  const status: Status = !browserOnline ? 'offline' : serverOnline ? 'connected' : 'disconnected';
  const label = t(STATUS_LABEL[status]);

  return (
    <Tooltip>
      <TooltipTrigger render={<span role="status" className={cn(TRIGGER_CLASS, className)} />}>
        <span className={cn('size-2 shrink-0 rounded-full', STATUS_DOT[status], status !== 'connected' && 'motion-safe:animate-pulse')} aria-hidden />
        {showLabel ? <span>{label}</span> : <span className="sr-only">{label}</span>}
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
