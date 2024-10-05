import { Dropzone } from '@mantine/dropzone';
import { useEffect, useState } from 'react';
import { FileWithPath } from 'react-dropzone';
import { useSWRConfig } from 'swr';
import { axiosInstance } from '../global.ts';

interface Props {
  url: string;
}

export default function AccountDocumentUpload(props: Props) {
  const [files, setFiles] = useState<FileWithPath[]>([]);
  const { mutate } = useSWRConfig();
  useEffect(() => {
    if (files.length > 0) {
      const formData = new FormData();
      files.forEach((file) => formData.append('file', file));
      axiosInstance
        .post(props.url, formData, {
          headers: {
            'Content-Type': 'multipart/form-data',
          },
          onUploadProgress: () => {
          },
        })
        .then(() => {
          setFiles([]);
          mutate(props.url);
        });
    }
  }, [files, props.url, mutate]);

  const filesDom = files.map((file: FileWithPath) => (
    <li key={file.path}>
      {file.path} - {file.size} bytes
    </li>
  ));

  return (
    <Dropzone onDrop={setFiles} radius="sm" maxSize={30 * 1024 ** 2}>
      {filesDom.length > 0 ? (
        <div>
          <ul>{filesDom}</ul>
        </div>
      ) : (
        <div style={{ pointerEvents: 'none' }}>
          <div className="text-center font-bold text-lg mt-4">
            <Dropzone.Accept>Drop files here</Dropzone.Accept>
            <Dropzone.Reject>Pdf file less than 30mb</Dropzone.Reject>
            <Dropzone.Idle>Upload Document</Dropzone.Idle>
          </div>
        </div>
      )}
    </Dropzone>
  );
}
