import { useDisclosure } from '@mantine/hooks';
import { useAtom } from 'jotai';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { previewJournalAtom } from '../../states/journals';
import JournalPreview from '../journalPreview/JournalPreview';
import { AutoDrawer, } from '../ui/auto-drawer';
import { Button } from '../ui/button';
import { DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog';

export const TransactionPreviewModal = () => {
  const [previewJournal, setPreviewJournal] = useAtom(previewJournalAtom);
  const { t } = useTranslation();
  const [isOpen, isOpenHandler] = useDisclosure(false);

  useEffect(() => {
    console.log('previewJournal changed', previewJournal);
    if (previewJournal !== undefined) {
      isOpenHandler.open();
    }
  }, [previewJournal]);

  const onChange = (b: boolean) => {
    if (b === false) {
      setPreviewJournal(undefined);
      isOpenHandler.close();
    }
  };

  return (
    <>
      <AutoDrawer open={isOpen} onOpenChange={onChange} title={t('TRANSACTION_PREVIEW_MODAL_TITLE')} description={t('TRANSACTION_PREVIEW_MODAL_DESCRIPTION')} footer={
        <Button variant="outline" onClick={() => onChange(false)}>
          {t('TRANSACTION_PREVIEW_MODAL_CLOSE')}
        </Button>
      }>
        <JournalPreview data={previewJournal} />
      </AutoDrawer>
    </>
  );
};
