import { useMutation, useQuery } from '@apollo/client';
import { Text, Button, Modal } from '@mantine/core';
import { ErrorEntity, ErrorListQuery, ERROR_LIST } from '../gql/errorList';
import { useState } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { MODIFY_FILE } from '../gql/modifyFile';
import { useTranslation } from 'react-i18next';
import { Stack } from '@mantine/core';

export default function ErrorBox() {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const { data, loading, refetch } = useQuery<ErrorListQuery>(ERROR_LIST);
  const [selectError, setSelectError] = useState<ErrorEntity | null>(null);
  const [selectErrorContent, setSelectErrorContent] = useState<string>('');

  const [modifyFile] = useMutation(MODIFY_FILE, {
    update: (proxy) => {
      proxy.evict({ fieldName: `journals` });
      proxy.evict({ fieldName: `errors` });
      console.log('proxy', proxy);
    },
  });

  const toggleError = (error: ErrorEntity) => {
    setSelectError(error);
    setSelectErrorContent(error.span.content);
    setIsOpen(true);
  };
  const saveErrorModfiyData = () => {
    modifyFile({
      variables: {
        file: selectError?.span.filename,
        content: selectErrorContent,
        start: selectError?.span.start,
        end: selectError?.span.end,
      },
    });
    setIsOpen(false);
  };
  const onModalReset = () => {
    setSelectErrorContent(selectError?.span.content || '');
  };
  const fetchNextPage = () => {
    refetch({
      cursor: data?.errors.pageInfo.endCursor,
    });
  };
  const fetchPreviousPage = () => {
    const cursor = parseInt(data!.errors.pageInfo.startCursor) - 11;
    if (cursor > 0) {
      refetch({
        cursor: cursor.toString(),
      });
    } else {
      refetch({ cursor: '-1' });
    }
  };

  if (loading) return <div> loading</div>;
  console.log('error', data);
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
      </Modal>
      <div>
        <Stack>
          {data?.errors.edges
            .map((edge) => edge.node)
            .map((error, idx) => (
              <Text onClick={() => toggleError(error)}>{error.message}</Text>
            ))}
        </Stack>

        <Button.Group>
          <Button disabled={!data?.errors.pageInfo.hasPreviousPage} onClick={fetchPreviousPage} variant="default">
            Previous
          </Button>
          <Button disabled={!data?.errors.pageInfo.hasNextPage} onClick={fetchNextPage} variant="default">
            Next
          </Button>
        </Button.Group>
      </div>
    </>
  );
}
