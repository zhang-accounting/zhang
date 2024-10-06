import { useCallback, useEffect, useState } from 'react';
import { FileWithPath } from 'react-dropzone';
import { useSWRConfig } from 'swr';
import { axiosInstance } from '../global.ts';
import {useDropzone} from 'react-dropzone'

interface Props {
  url: string;
}

export default function AccountDocumentUpload(props: Props) {
  const [files, setFiles] = useState<FileWithPath[]>([]);
  const { mutate } = useSWRConfig();



  const onDrop = useCallback((acceptedFiles: FileWithPath[]) => {
    setFiles(acceptedFiles);
  }, [])

  const {getRootProps, getInputProps, isDragActive} = useDropzone({onDrop})

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

  return (

    <div {...getRootProps()} className="relative overflow-hidden rounded-md after:content-[''] after:block after:pb-[100%] bg-gray-100 dark:bg-dark-700">
      <input {...getInputProps()} />
      {
        isDragActive ?
        <div className="absolute w-full h-full flex flex-col items-center justify-center text-center p-4">
        <p>Drop the files here ...</p> :
        </div>
        : <div className="cursor-pointer absolute w-full h-full flex flex-col items-center justify-center text-center p-4">
        <p>Drag 'n' drop some files here, or click to select files</p>
        </div>
      }
    </div>
  );
}
