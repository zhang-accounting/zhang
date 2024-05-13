import { Button, Group, Modal, Pagination, Stack, Text, Textarea } from '@mantine/core';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { LoadingState } from '../rest-model';
import { useAppDispatch, useAppSelector } from '../states';
import { fetchError, LedgerError } from '../states/errors';
import { ErrorsSkeleton } from './skeletons/errorsSkeleton';

export default function ErrorBox() {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);

  const dispatch = useAppDispatch();
  const { items, total_page, status } = useAppSelector((state) => state.errors);

  const [page, setPage] = useState(1);

  const [selectError, setSelectError] = useState<LedgerError | null>(null);
  const [selectErrorContent, setSelectErrorContent] = useState<string>('');

  if (status === LoadingState.Loading || status === LoadingState.NotReady) {
    return <ErrorsSkeleton />;
  }
  const handlePageChange = (newPage: number) => {
    setPage(newPage);
    dispatch(fetchError(newPage));
  };

  const toggleError = (error: LedgerError) => {
    setSelectError(error);
    setSelectErrorContent(error.span.content);
    setIsOpen(true);
  };

  const saveErrorModfiyData = () => {
    //   modifyFile({
    //     variables: {
    //       file: selectError?.span.filename,
    //       content: selectErrorContent,
    //       start: selectError?.span.start,
    //       end: selectError?.span.end,
    //     },
    //   });
    setIsOpen(false);
  };
  const onModalReset = () => {
    setSelectErrorContent(selectError?.span.content || '');
  };
  return (
    <>
      <Modal
        size="lg"
        centered
        opened={isOpen}
        onClose={() => setIsOpen(false)}
        title={`${selectError?.span.filename}:${selectError?.span.start}:${selectError?.span.end}`}
      >
        <Text>{t(`ERROR.${selectError?.error_type || ''}`)}</Text>
        <Textarea
          value={selectErrorContent}
          onChange={(event) => {
            setSelectErrorContent(event.target.value);
          }}
        />
        <Group justify="right">
          <Button onClick={onModalReset} variant="default">
            {t('RESET')}
          </Button>
          <Button onClick={saveErrorModfiyData} variant="default">
            {t('SAVE')}
          </Button>
        </Group>
      </Modal>
      <Stack>
        {items.map((error, idx) => (
          <Text key={idx} onClick={() => toggleError(error)}>
            {t(`ERROR.${error.error_type}`)}
          </Text>
        ))}

        <Group justify="center">
          <Pagination mt="xs" total={total_page} value={page} onChange={handlePageChange} />
        </Group>
      </Stack>
    </>
  );
}
