import { cn } from '@/lib/utils';

interface Props {
  payee?: string | null;
  narration?: string | null;
  onClick?: () => void;
  className?: string;
}

/** `Payee · Narration` on one truncated line; the payee is emphasised. */
export default function PayeeNarration({ payee, narration, onClick, className }: Props) {
  const content = (
    <>
      {payee && <span className="font-medium text-foreground">{payee}</span>}
      {payee && narration && <span className="px-1.5 text-muted-foreground">·</span>}
      {narration && <span>{narration}</span>}
    </>
  );
  if (onClick) {
    return (
      <button type="button" className={cn('min-w-0 truncate text-left hover:underline hover:underline-offset-4', className)} onClick={onClick}>
        {content}
      </button>
    );
  }
  return <span className={cn('block min-w-0 truncate', className)}>{content}</span>;
}
