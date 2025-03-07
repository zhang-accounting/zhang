import { retrieveFile, updateFile } from '@/api/requests.ts';
import CodeMirror from '@uiw/react-codemirror';
import { Buffer } from 'buffer';
import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button } from './ui/button.tsx';
import { useAsync } from 'react-use';

interface Props {
  name?: string;
  path: string;
}

export default function SingleFileEdit({ path }: Props) {
  let encodedPath = Buffer.from(path).toString('base64');

  const { value: data, error } = useAsync(async () => {
    const response = await retrieveFile({ file_path: encodedPath });
    return response.data.data;
  });

  const onUpdate = async () => {
    await updateFile({
      file_path: path,
      content: content,
    });
    toast.success('File updated', {
      description: 'Ledger will be refreshed in a moment',
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
      <CodeMirror
        value={content}
        height="85vh"
        width="100%"
        onChange={(value) => {
          setContent(value);
        }}
      />
      <Button className="ml-2 mt-2" disabled={data.content === content} onClick={onUpdate}>
        Update
      </Button>
    </div>
  );
}
