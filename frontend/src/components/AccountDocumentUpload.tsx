import { Text } from '@mantine/core';
import { Dropzone } from '@mantine/dropzone';
import { useEffect, useState } from 'react';
import { FileWithPath } from 'react-dropzone';
import { axiosInstance } from '../index';

interface Props {
  url: string;
}

export default function AccountDocumentUpload(props: Props) {
  const [files, setFiles] = useState<FileWithPath[]>([]);

  useEffect(() => {
    console.log('files', files);
    if (files.length > 0) {
      const formData = new FormData();
      files.forEach((file) => formData.append('file', file));

      axiosInstance
        .post(props.url, formData, {
          headers: {
            'Content-Type': 'multipart/form-data',
          },
          onUploadProgress: () => {},
        })
        .then(() => {
          setFiles([]);
        });
    }
  }, [files, props.url]);

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
          <Text align="center" weight={700} size="lg" mt="md">
            <Dropzone.Accept>Drop files here</Dropzone.Accept>
            <Dropzone.Reject>Pdf file less than 30mb</Dropzone.Reject>
            <Dropzone.Idle>Upload Documents</Dropzone.Idle>
          </Text>
          <Text align="center" size="sm" mt="xs" color="dimmed">
            Drag&apos;n&apos;drop files here to upload. We can accept only <i>.pdf</i> files that are less than 30mb in size.
          </Text>
        </div>
      )}
    </Dropzone>
  );
}
