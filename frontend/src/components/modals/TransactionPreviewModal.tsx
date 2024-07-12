import { ContextModalProps } from '@mantine/modals';
import JournalPreview from '../journalPreview/JournalPreview';
import { loadable_unwrap } from '../../states';
import { useAtomValue } from 'jotai';
import { useMemo } from 'react';
import { selectAtom } from 'jotai/utils';
import { journalAtom } from '../../states/journals';

export const TransactionPreviewModal = ({ context, id, innerProps }: ContextModalProps<{ journalId: string }>) => {
  const targetJournal = useAtomValue(
    useMemo(
      () =>
        selectAtom(journalAtom, (val) =>
          loadable_unwrap(val, undefined, (data) => data.records.find((journalItem) => journalItem.id === innerProps.journalId)),
        ),
      [innerProps],
    ),
  );

  return (
    <>
      <JournalPreview data={targetJournal} />
    </>
  );
};
