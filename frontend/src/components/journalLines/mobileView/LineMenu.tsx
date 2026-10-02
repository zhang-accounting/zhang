import { Ellipsis } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu';

interface Props {
  actions: {
    label: string;
    icon: React.ElementType;
    onClick: () => void;
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
            <DropdownMenuItem key={index} onClick={action.onClick}>
              <action.icon />
              {action.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
