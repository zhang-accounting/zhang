import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import { Pencil } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router';
import { JournalItem } from '@/api/types';
import { editTransactionAtom, journalAtom, previewJournalAtom, refetchJournal } from '../../states/journals';
import { transactionEditBlocker } from '../journalLines/journal-utils';
import JournalPreview from '../journalPreview/JournalPreview';
import { AutoDrawer } from '../ui/auto-drawer';
import { Button } from '../ui/button';
import { RawEditNote } from './RawEditNote';

/** Journal detail: Dialog >= md, bottom Drawer < md. Mount next to `TransactionEditModal` so "Edit" can hand over. */
export const TransactionPreviewModal = () => {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  const [previewJournal, setPreviewJournal] = useAtom(previewJournalAtom);
  const setEditTransaction = useSetAtom(editTransactionAtom);
  const journals = useAtomValue(journalAtom);
  // Keep the last journal while the close animation runs.
  const [shown, setShown] = useState<JournalItem | undefined>(previewJournal);

  useEffect(() => {
    if (previewJournal !== undefined) setShown(previewJournal);
  }, [previewJournal]);

  // The atom is global: never carry an open preview over to another page (or back to this one later).
  useEffect(() => () => setPreviewJournal(undefined), [pathname, setPreviewJournal]);

  // Whenever the shared journal list is refreshed (document upload, edit, SSE ledger reload), re-read the open transaction so
  // the preview shows its new documents / values instead of the snapshot taken when it was opened.
  useEffect(() => {
    if (journals.state !== 'hasData' || previewJournal?.type !== 'Transaction') return;
    let cancelled = false;
    const target = previewJournal;
    const inList = journals.data.records.find((record) => record.id === target.id);
    (inList ? Promise.resolve(inList) : refetchJournal(target))
      .then((fresh) => {
        if (!cancelled && fresh) setPreviewJournal((current) => (current?.id === fresh.id ? fresh : current));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [journals]);

  const onOpenChange = (open: boolean) => {
    if (!open) setPreviewJournal(undefined);
  };

  const editBlocked = shown?.type === 'Transaction' && transactionEditBlocker(shown) !== null;

  const onEdit = () => {
    if (shown?.type !== 'Transaction' || editBlocked) return;
    setPreviewJournal(undefined);
    setEditTransaction(shown);
  };

  return (
    <AutoDrawer
      open={previewJournal !== undefined}
      onOpenChange={onOpenChange}
      className="sm:max-w-xl md:max-h-[90vh] md:overflow-y-auto"
      title={t('TRANSACTION_PREVIEW_MODAL_TITLE')}
      description={t('ledger.preview.description')}
      footer={
        <>
          <Button variant="outline" className="h-10 md:h-8" onClick={() => onOpenChange(false)}>
            {t('TRANSACTION_PREVIEW_MODAL_CLOSE')}
          </Button>
          {shown?.type === 'Transaction' && (
            <Button className="h-10 md:h-8" onClick={onEdit} disabled={editBlocked} aria-describedby={editBlocked ? 'txn-edit-blocked-note' : undefined}>
              <Pencil data-icon="inline-start" />
              {t('ledger.journal.edit')}
            </Button>
          )}
        </>
      }
    >
      {editBlocked && <RawEditNote id="txn-edit-blocked-note" className="mt-4 md:mt-0" />}
      <JournalPreview data={shown} />
    </AutoDrawer>
  );
};
