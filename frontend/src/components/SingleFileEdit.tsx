import { gql, useMutation, useQuery } from '@apollo/client';
import { Button } from '@mantine/core';
import CodeMirror from '@uiw/react-codemirror';
import { useEffect, useState } from 'react';
import useSWR, { useSWRConfig } from 'swr';
import { fetcher } from '..';
import { SingleFileEntryQuery, SINGLE_FILE_ENTRY } from '../gql/singleFile';
import { Buffer } from 'buffer'
import axios from 'axios';
interface Props {
  name?: string;
  path: string;
}

export default function SingleFileEdit({ path }: Props) {
  const { mutate } = useSWRConfig()

  let encodedPath = Buffer.from(path).toString('base64');
  const { data, error } = useSWR<{ content: string, path: string }>(`/api/files/${encodedPath}`, fetcher)
  const [update] = useMutation(
    gql`
      mutation UPDATE_FILE($path: String, $content: String) {
        updateFile(path: $path, content: $content)
      }
    `,
    {
      refetchQueries: ['FILE_LIST', 'SINGLE_FILE_ENTRY', 'JOURNAL_LIST', 'BAR_STATISTIC'],
    },
  );
  const onUpdate = () => {
    axios.put(`/api/files/${encodedPath}`, {
      content: content,
    })
    .then(function (response) {
      mutate(`/api/files/${encodedPath}`)
    })
    .catch(function (error) {
      console.log(error);
    });
  }
  
  const [content, setContent] = useState('');

  useEffect(() => {
    setContent(data?.content || '');
  }, [data]);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <div>
      <CodeMirror
        value={content}
        height="85vh"
        width="100%"
        onChange={(value) => {
          setContent(value);
        }}
      />
      <Button disabled={data.content === content} onClick={onUpdate}>
        Update
      </Button>
    </div>
  );
}
