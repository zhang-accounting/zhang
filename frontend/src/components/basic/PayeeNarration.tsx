import { cn } from '@/lib/utils';

interface Props {
  payee?: string | null;
  narration?: string | null;
  onClick?: () => void;
}

export default function PayeeNarration(props: Props) {
  return (
    <div
      className={cn(
        'flex items-center gap-2 px-2 py-1 border border-transparent rounded-md',
        props.onClick && 'cursor-pointer hover:border-dashed hover:border-foreground/10',
      )}
      onClick={props.onClick}
    >
      {props.payee && <span className="font-bold after:content-['·'] after:font-bold after:ml-2">{props.payee}</span>}
      <span>{props.narration ?? ''}</span>
    </div>
  );
}
