import { FileText, Link2 } from 'lucide-react';
import { Link } from 'react-router';
import { Document } from '@/api/types';
import { useDateFormat } from '@/components/layout/use-date-format';
import { cn } from '@/lib/utils';
import { isDocumentAnImage } from '../../utils/documents';
import { documentExtension, documentUrl } from './document-utils';

export interface Props extends Document {
  onClick: (path: string) => void;
  className?: string;
}

/** Thumbnail card for the documents grid: 4:3 preview (image or file-type tile), file name and link. */
export default function AccountDocumentLine(props: Props) {
  const fmt = useDateFormat();
  const canPreview = isDocumentAnImage(props.path);
  const previewClass =
    'relative block aspect-[4/3] w-full overflow-hidden bg-muted outline-none focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:ring-inset';

  return (
    <figure className={cn('group flex min-w-0 flex-col overflow-hidden rounded-xl border bg-card', props.className)}>
      {canPreview ? (
        <button type="button" className={cn(previewClass, 'cursor-zoom-in')} onClick={() => props.onClick(props.path)} aria-label={props.filename}>
          <img
            className="absolute inset-0 size-full object-cover transition-transform duration-200 group-hover:scale-[1.02]"
            alt={props.filename}
            loading="lazy"
            src={documentUrl(props.path)}
          />
        </button>
      ) : (
        <a href={documentUrl(props.path)} target="_blank" rel="noreferrer" className={cn(previewClass, 'flex flex-col items-center justify-center gap-2')}>
          <FileText className="size-8 text-muted-foreground" />
          <span className="rounded-md bg-background px-1.5 py-0.5 text-[11px] font-semibold tracking-wide text-muted-foreground">
            {documentExtension(props) || 'FILE'}
          </span>
        </a>
      )}
      <figcaption className="flex min-w-0 flex-col gap-0.5 p-2.5">
        <span className="truncate text-sm font-medium" title={props.filename}>
          {props.filename}
        </span>
        <span className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
          <span className="shrink-0 tabular-nums">{fmt.day(new Date(props.datetime))}</span>
          {props.account && (
            <>
              <span aria-hidden>·</span>
              <Link to={`/accounts/${props.account}`} className="truncate hover:text-foreground hover:underline" title={props.account}>
                {props.account}
              </Link>
            </>
          )}
          {!props.account && props.trx_id && (
            <>
              <span aria-hidden>·</span>
              <Link2 className="size-3 shrink-0" />
              <span className="truncate font-mono" title={props.trx_id}>
                {props.trx_id.slice(0, 8)}
              </span>
            </>
          )}
        </span>
      </figcaption>
    </figure>
  );
}
