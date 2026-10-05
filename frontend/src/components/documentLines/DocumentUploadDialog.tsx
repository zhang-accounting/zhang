import { useAtomValue } from 'jotai';
import { FileUp, Upload, X } from 'lucide-react';
import { useId, useState } from 'react';
import { FileWithPath, useDropzone } from 'react-dropzone';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';
import { uploadDocuments } from '@/api/requests';
import { GroupCombobox } from '@/components/basic/GroupCombobox';
import { AutoDrawer, AutoDrawerTrigger } from '@/components/ui/auto-drawer';
import { Button } from '@/components/ui/button';
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field';
import { apiErrorMessage } from '@/lib/api-error';
import { cn } from '@/lib/utils';
import { documentAccountSelectItemsAtom } from '@/states/account';

interface Props {
  onUploaded?: () => void;
}

/** "Upload" action for the documents page: pick an account, drop files, upload them as account documents. */
export function DocumentUploadDialog({ onUploaded }: Props) {
  const { t } = useTranslation();
  const id = useId();
  const [open, setOpen] = useState(false);
  const [account, setAccount] = useState<string | undefined>();
  const [files, setFiles] = useState<FileWithPath[]>([]);
  const [uploading, setUploading] = useState(false);
  // a document only records: it may name a closed account, as a final statement does
  const accountItems = useAtomValue(documentAccountSelectItemsAtom);
  const { getRootProps, getInputProps, isDragActive } = useDropzone({ onDrop: (accepted) => setFiles((current) => [...current, ...accepted]) });

  const reset = () => {
    setFiles([]);
    setAccount(undefined);
  };

  const onSubmit = async () => {
    if (!account || files.length === 0) return;
    setUploading(true);
    try {
      await uploadDocuments('accounts', account, files);
      toast.success(t('documents.upload_success', { count: files.length }));
      reset();
      setOpen(false);
      onUploaded?.();
    } catch (error) {
      toast.error(t('documents.upload_failed'), { description: await apiErrorMessage(error) });
    } finally {
      setUploading(false);
    }
  };

  return (
    <AutoDrawer
      open={open}
      onOpenChange={setOpen}
      title={t('documents.upload_title')}
      description={t('documents.upload_description')}
      className="sm:max-w-lg"
      footer={
        <>
          <Button variant="outline" className="h-10 md:h-8" onClick={() => setOpen(false)}>
            {t('documents.cancel')}
          </Button>
          <Button className="h-10 md:h-8" disabled={!account || files.length === 0 || uploading} onClick={onSubmit}>
            <Upload />
            {uploading ? t('documents.uploading') : t('documents.upload_submit', { count: files.length })}
          </Button>
        </>
      }
    >
      <AutoDrawerTrigger render={<Button className="h-10 md:h-8" />}>
        <Upload />
        {t('documents.upload')}
      </AutoDrawerTrigger>
      <FieldGroup className="gap-4 pb-1">
        <Field>
          <FieldLabel htmlFor={`${id}-account`}>{t('documents.account')}</FieldLabel>
          <GroupCombobox
            id={`${id}-account`}
            placeholder={t('documents.account_placeholder')}
            options={accountItems}
            value={account}
            onChange={setAccount}
            className="h-10 md:h-8"
          />
        </Field>
        <Field>
          <FieldLabel id={`${id}-files`}>{t('documents.files')}</FieldLabel>
          <div
            {...getRootProps({ role: 'button', 'aria-labelledby': `${id}-files`, 'aria-describedby': `${id}-files-hint` })}
            className={cn(
              'flex min-h-32 cursor-pointer flex-col items-center justify-center gap-2 rounded-xl border border-dashed p-4 text-center',
              'transition-colors outline-none',
              'hover:bg-muted/50 focus-visible:ring-3 focus-visible:ring-ring/50',
              isDragActive && 'border-link bg-primary/5',
            )}
          >
            <input {...getInputProps()} />
            <FileUp className="size-6 text-muted-foreground" />
            <span className="text-sm font-medium">{isDragActive ? t('documents.drop_active') : t('documents.drop_hint')}</span>
            <FieldDescription id={`${id}-files-hint`} className="text-xs">
              {t('documents.drop_description')}
            </FieldDescription>
          </div>
          {files.length > 0 && (
            <ul className="flex flex-col divide-y rounded-lg border">
              {files.map((file, index) => (
                <li key={`${file.name}-${index}`} className="flex items-center gap-2 py-1 pr-1 pl-3 text-sm">
                  <span className="min-w-0 flex-1 truncate">{file.name}</span>
                  <span className="shrink-0 text-xs text-muted-foreground tabular-nums">{Math.max(1, Math.round(file.size / 1024))} KB</span>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-10 md:size-7"
                    aria-label={t('documents.remove_file')}
                    onClick={() => setFiles((current) => current.filter((_, i) => i !== index))}
                  >
                    <X />
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </Field>
      </FieldGroup>
    </AutoDrawer>
  );
}
