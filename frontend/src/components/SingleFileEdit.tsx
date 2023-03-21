import { Button, Textarea } from '@mantine/core';
import axios from 'axios';
import { Buffer } from 'buffer';
import { useEffect, useState } from 'react';
import useSWR, { useSWRConfig } from 'swr';
import { fetcher, serverBaseUrl } from '..';

interface Props {
  name?: string;
  path: string;
}

export default function SingleFileEdit({ path }: Props) {
  const { mutate } = useSWRConfig();

  let encodedPath = Buffer.from(path).toString('base64');
  const { data, error } = useSWR<{ content: string; path: string }>(`/api/files/${encodedPath}`, fetcher);

  const onUpdate = () => {
    axios
      .put(`${serverBaseUrl}/api/files/${encodedPath}`, {
        content: content,
      })
      .then(function (response) {
        mutate(`/api/files/${encodedPath}`);
      })
      .catch(function (error) {
        console.log(error);
      });
  };

  const [content, setContent] = useState('');

  useEffect(() => {
    setContent(data?.content || '');
  }, [data]);

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <div>
      <Textarea ml="xs" minRows={30} value={content} onChange={(event) => setContent(event.target.value)} />
      <Button ml="xs" mt="xs" disabled={data.content === content} onClick={onUpdate}>
        Update
      </Button>
    </div>
  );
}
