import { uploadAccountDocument, uploadTransactionDocument } from '@/api/requests';
import { useCallback, useEffect, useState } from 'react';
import { FileWithPath, useDropzone } from 'react-dropzone';

interface Props {
  type: 'transaction' | 'account';
  id: string;
}

export default function AccountDocumentUpload(props: Props) {
  const [files, setFiles] = useState<FileWithPath[]>([]);

  const onDrop = useCallback((acceptedFiles: FileWithPath[]) => {
    setFiles(acceptedFiles);
  }, []);

  const { getRootProps, getInputProps, isDragActive } = useDropzone({ onDrop });

  const sendRequest = async (id: string, formData: FormData) => {
    if (props.type === 'transaction') {
      await uploadTransactionDocument({
        //@ts-ignore
        transaction_id: id,
        //@ts-ignore
        file: formData,
      });
    }
    if (props.type === 'account') {
      await uploadAccountDocument({
        //@ts-ignore
        account_name: id,
        //@ts-ignore
        file: formData,
      });
    }
  };

  useEffect(() => {
    if (files.length > 0) {
      const formData = new FormData();
      files.forEach((file) => formData.append('file', file));
      sendRequest(props.id, formData).then(() => {
        setFiles([]);
      });
    }
  }, [files, props.id, props.type]);

  return (
    <div {...getRootProps()} className="relative overflow-hidden rounded-md after:content-[''] after:block after:pb-[100%] bg-gray-100 dark:bg-dark-700">
      <input {...getInputProps()} />
      {isDragActive ? (
        <div className="absolute w-full h-full flex flex-col items-center justify-center text-center p-4">
          <p>Drop the files here ...</p> :
        </div>
      ) : (
        <div className="cursor-pointer absolute w-full h-full flex flex-col items-center justify-center text-center p-4">
          <p>Drag 'n' drop some files here, or click to select files</p>
        </div>
      )}
    </div>
  );
}
