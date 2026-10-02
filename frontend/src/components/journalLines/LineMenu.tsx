import { Ellipsis } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';
import { cn } from '@/lib/utils';

interface Props {
  actions: {
    label: string;
    icon: React.ElementType;
    onClick: () => void;
    disabled?: boolean;
    /** Second line under the label, e.g. why the action is disabled. */
    hint?: string;
  }[];
}

/** Row "…" menu. Clicks are stopped here so they never reach the clickable row (React events bubble through portals). */
export function LineMenu(props: Props) {
  const { t } = useTranslation();
  return (
    <div className="inline-flex" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => event.stopPropagation()}>
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button variant="ghost" size="icon" className="size-10 md:size-8" aria-label={t('ledger.journal.actions')} />}>
          <Ellipsis />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-36">
          {props.actions.map((action, index) => (
            <DropdownMenuItem key={index} onClick={action.onClick} disabled={action.disabled} className={cn(action.hint && 'items-start')}>
              <action.icon className={cn(action.hint && 'mt-0.5')} />
              {action.hint ? (
                <span className="flex max-w-56 flex-col">
                  <span>{action.label}</span>
                  <span className="text-xs text-muted-foreground">{action.hint}</span>
                </span>
              ) : (
                action.label
              )}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
