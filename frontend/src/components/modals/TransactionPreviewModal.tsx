import JournalPreview from '../journalPreview/JournalPreview';
import { useAtom } from 'jotai';
import { useEffect } from 'react';
import { previewJournalAtom } from '../../states/journals';
import { Button } from '../ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog';
import { useTranslation } from 'react-i18next';
import { useDisclosure } from '@mantine/hooks';

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
  }

  return (
    <>
      <Dialog open={isOpen} onOpenChange={onChange}  >
        <DialogContent className="sm:max-w-[600px]">
          <DialogHeader>
            <DialogTitle>{t('TRANSACTION_PREVIEW_MODAL_TITLE')}</DialogTitle>
            <DialogDescription hidden>
              {t('TRANSACTION_PREVIEW_MODAL_DESCRIPTION')}
            </DialogDescription>
          </DialogHeader>
          <JournalPreview data={previewJournal} />

          <DialogFooter>
            <Button variant="outline" onClick={() => onChange(false)}>
              {t('TRANSACTION_PREVIEW_MODAL_CLOSE')}
            </Button>

          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
};
