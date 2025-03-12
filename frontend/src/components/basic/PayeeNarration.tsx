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
        'flex items-center gap-2 md:px-2 md:py-1 md:border border-transparent rounded-md line-clamp-1',
        props.onClick && 'cursor-pointer md:hover:border-dashed hover:border-foreground/10 ',
      )}
      onClick={props.onClick}
    >
      {props.payee && <span className="font-bold after:content-['·'] after:font-bold after:ml-2">{props.payee}</span>}
      <span>{props.narration ?? ''}</span>
    </div>
  );
}
