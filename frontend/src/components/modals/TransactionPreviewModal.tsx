import { ContextModalProps } from '@mantine/modals';
import JournalPreview from '../journalPreview/JournalPreview';
import { useAppSelector } from '../../states';

export const TransactionPreviewModal = ({ context, id, innerProps }: ContextModalProps<{ journalId: string }>) => {
  const targetJournal = useAppSelector((state) => (state.journals.items ?? []).find((journalItem) => journalItem.id === innerProps.journalId));

  return (
    <>
      <JournalPreview data={targetJournal} />
    </>
  );
};
