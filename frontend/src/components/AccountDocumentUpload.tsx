import { Upload } from 'lucide-react';
import { useCallback, useState } from 'react';
import { FileWithPath, useDropzone } from 'react-dropzone';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { uploadDocuments } from '@/api/requests';
import { apiErrorMessage } from '@/lib/api-error';
import { cn } from '@/lib/utils';
import { Spinner } from './ui/spinner';

interface Props {
  type: 'transaction' | 'account';
  id: string;
  /** Called after a successful upload (e.g. to reload the document list). */
  onUploaded?: () => void;
  className?: string;
}

/** Square drop zone tile that uploads documents to an account or a transaction. */
export default function AccountDocumentUpload({ type, id, onUploaded, className }: Props) {
  const { t } = useTranslation();
  const [uploading, setUploading] = useState(false);

  const onDrop = useCallback(
    async (files: FileWithPath[]) => {
      if (files.length === 0) return;
      setUploading(true);
      try {
        await uploadDocuments(type === 'transaction' ? 'transactions' : 'accounts', id, files);
        toast.success(t('ledger.documents.uploaded', { count: files.length }));
        onUploaded?.();
      } catch (error) {
        toast.error(t('ledger.documents.upload_failed'), { description: await apiErrorMessage(error) });
      } finally {
        setUploading(false);
      }
    },
    [type, id, onUploaded, t],
  );

  const { getRootProps, getInputProps, isDragActive } = useDropzone({ onDrop, disabled: uploading });

  return (
    <div
      {...getRootProps()}
      className={cn(
        'flex aspect-square cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border border-dashed p-3 text-center',
        'bg-muted/30 text-xs text-muted-foreground transition-colors outline-none hover:bg-muted/60 focus-visible:ring-3 focus-visible:ring-ring/50',
        isDragActive && 'border-link bg-primary/5 text-foreground',
        className,
      )}
    >
      <input {...getInputProps()} />
      {uploading ? <Spinner className="size-5" /> : <Upload className="size-5" aria-hidden />}
      <span>{isDragActive ? t('ledger.documents.drop_here') : t('ledger.documents.upload_hint')}</span>
    </div>
  );
}
