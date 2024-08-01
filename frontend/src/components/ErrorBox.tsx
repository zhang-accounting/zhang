import { Anchor, Button, Group, Image, Modal, Pagination, Stack, Text, Textarea } from '@mantine/core';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { errorAtom, errorPageAtom, LedgerError } from '../states/errors';
import { ErrorsSkeleton } from './skeletons/errorsSkeleton';
import { useAtomValue, useSetAtom } from 'jotai';
import Joyride from '../assets/joyride.svg';

export default function ErrorBox() {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);

  const [selectError, setSelectError] = useState<LedgerError | null>(null);
  const [selectErrorContent, setSelectErrorContent] = useState<string>('');

  const errors = useAtomValue(errorAtom);
  const setErrorPage = useSetAtom(errorPageAtom);

  if (errors.state === 'loading' || errors.state === 'hasError') {
    return <ErrorsSkeleton />;
  }
  const handlePageChange = (newPage: number) => {
    setErrorPage(newPage);
  };

  const toggleError = (error: LedgerError) => {
    setSelectError(error);
    setSelectErrorContent(error.span.content);
    setIsOpen(true);
  };

  const saveErrorModifyData = () => {
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
        title={`${selectError?.span.filename}:L${selectError?.span.start}:${selectError?.span.end}`}
      >
        <Text>{t(`ERROR.${selectError?.error_type || ''}`)}</Text>
        <Textarea
          value={selectErrorContent}
          onChange={(event) => {
            setSelectErrorContent(event.target.value);
          }}
        />
        <Group justify="space-between" mt={'lg'}>
          <Group>
            <Anchor href={`https://zhang-accounting.kilerd.me/user-guide/error-code/#${selectError?.error_type.toLocaleLowerCase()}`} target="_blank">
              {t('ERROR_BOX_WHY')}
            </Anchor>
          </Group>
          <Group>
            <Button onClick={onModalReset} variant="default">
              {t('RESET')}
            </Button>
            <Button onClick={saveErrorModifyData} variant="default">
              {t('SAVE')}
            </Button>
          </Group>
        </Group>
      </Modal>
      <Stack gap={'xs'}>
        {errors.data.total_count === 0 ? (
          <Stack align={'center'}>
            <Image radius="md" w={'85%'} src={Joyride} />
            <Text>{t('LEDGER_IS_HEALTHY')}</Text>
          </Stack>
        ) : (
          <>
            {errors.data.records.map((error, idx) => (
              <Text key={idx} onClick={() => toggleError(error)}>
                {t(`ERROR.${error.error_type}`)}
              </Text>
            ))}

            <Group justify="center">
              <Pagination size="sm" mt="xs" total={errors.data.total_page} value={errors.data.current_page} onChange={handlePageChange} />
            </Group>
          </>
        )}
      </Stack>
    </>
  );
}
