import { Buffer } from 'buffer';
import { FileText } from 'lucide-react';
import { cn } from '@/lib/utils';
import { isDocumentAnImage } from '../../utils/documents';

interface Props {
  filename: string;
  onClick: (path: string) => void;
  className?: string;
}

const documentUrl = (path: string) => `/api/documents/${Buffer.from(path).toString('base64')}`;

/** Square document tile: image thumbnail (opens the lightbox) or a file card that opens the document in a new tab. */
export default function DocumentPreview({ filename, onClick, className }: Props) {
  const name = filename.split('/').pop() ?? filename;
  const tileClass = cn(
    'group relative flex aspect-square overflow-hidden rounded-lg border bg-muted/30 outline-none focus-visible:ring-3 focus-visible:ring-ring/50',
    className,
  );

  if (isDocumentAnImage(filename)) {
    return (
      <button type="button" className={tileClass} onClick={() => onClick(filename)} title={name}>
        <img className="size-full object-cover transition-transform group-hover:scale-[1.02]" alt={name} src={documentUrl(filename)} loading="lazy" />
      </button>
    );
  }
  return (
    <a
      className={cn(tileClass, 'flex-col items-center justify-center gap-2 p-3 text-center hover:bg-muted/60')}
      href={documentUrl(filename)}
      target="_blank"
      rel="noreferrer"
      title={name}
    >
      <FileText className="size-6 text-muted-foreground" aria-hidden />
      <span className="line-clamp-2 text-xs break-all">{name}</span>
    </a>
  );
}
