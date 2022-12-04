import { useMutation, useQuery } from '@apollo/client';
import { Text, Button, Modal, Group } from '@mantine/core';
import { ErrorEntity, ErrorListQuery, ERROR_LIST } from '../gql/errorList';
import { useState } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { MODIFY_FILE } from '../gql/modifyFile';
import { useTranslation } from 'react-i18next';
import { Stack } from '@mantine/core';

export default function ErrorBox() {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  // const { data, loading, refetch } = useQuery<ErrorListQuery>(ERROR_LIST);
  const data: ErrorEntity[] = [];
  const [selectError, setSelectError] = useState<ErrorEntity | null>(null);
  const [selectErrorContent, setSelectErrorContent] = useState<string>('');

  // const [modifyFile] = useMutation(MODIFY_FILE, {
  //   update: (proxy) => {
  //     proxy.evict({ fieldName: `journals` });
  //     proxy.evict({ fieldName: `errors` });
  //     console.log('proxy', proxy);
  //   },
  // });
  //
  const toggleError = (error: ErrorEntity) => {
    // setSelectError(error);
    // setSelectErrorContent(error.span.content);
    // setIsOpen(true);
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
    // setSelectErrorContent(selectError?.span.content || '');
  };
  // const fetchNextPage = () => {
  //   refetch({
  //     cursor: data?.errors.pageInfo.endCursor,
  //   });
  // };
  // const fetchPreviousPage = () => {
  //   const cursor = parseInt(data!.errors.pageInfo.startCursor) - 11;
  //   if (cursor > 0) {
  //     refetch({
  //       cursor: cursor.toString(),
  //     });
  //   } else {
  //     refetch({ cursor: '-1' });
  //   }
  // };


  return (
    <>
      <Modal
        size="lg"
        centered
        opened={isOpen}
        onClose={() => setIsOpen(false)}
        title={`${selectError?.span.filename}:${selectError?.span.start}:${selectError?.span.end}`}>
        <Text>{selectError?.message}</Text>
        <CodeMirror
          value={selectErrorContent}
          height="20vh"
          width="100%"
          onChange={(value) => {
            setSelectErrorContent(value);
          }}
        />
        <Button onClick={onModalReset} variant="default">
          {t('Reset')}
        </Button>
        <Button onClick={saveErrorModfiyData} variant="default">
          {t('Save')}
        </Button>
      </Modal>
      <Stack>
        <Stack>
          {data
            .map((error, idx) => (
              <Text key={idx} onClick={() => toggleError(error)} lineClamp={1}>
                {error.message}
              </Text>
            ))}
        </Stack>
        <Group position="right">
          <Button.Group>
            <Button  variant="default">
              {t('Previous')}
            </Button>
            <Button  variant="default">
              {t('Next')}
            </Button>
          </Button.Group>
        </Group>
      </Stack>
    </>
  );
}
