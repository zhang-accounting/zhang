import { ReactNode } from 'react';
import { Button } from '../ui/button';

interface Props {
  value: string;
  icon?: ReactNode;
  onActionClick?(): void;
}

/** Small section label with a hairline and an optional trailing icon action. */
export default function DividerWithAction({ value, icon, onActionClick }: Props) {
  return (
    <div className="my-2 flex items-center gap-2">
      <span className="shrink-0 text-xs font-medium text-muted-foreground">{value}</span>
      <div className="h-px grow bg-border" />
      {icon && (
        <Button variant="ghost" size="icon" className="size-10 md:size-8" aria-label={value} onClick={onActionClick}>
          {icon}
        </Button>
      )}
    </div>
  );
}
