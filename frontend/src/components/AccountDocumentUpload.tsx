import { useMutation } from '@apollo/client';
import React, { useEffect } from 'react';
import { useDropzone, FileWithPath } from 'react-dropzone';
import { UPLOAD_ACCOUNT_DOCUMENT } from '../gql/uploadAccountDocument';

interface Props {
    accountName: string
}
export default function AccountDocumentUpload(props: Props) {
    const { acceptedFiles, getRootProps, getInputProps } = useDropzone();
    
    const [uploadAccountDocuments] = useMutation(UPLOAD_ACCOUNT_DOCUMENT, {
        refetchQueries: [],
        
        update: (proxy)=> {
            proxy.evict({fieldName:`account({"name":"${props.accountName}"})`});
            proxy.evict({fieldName:`journals`});
        }
    });
    useEffect(() => {
        console.log("files", acceptedFiles);
        if(acceptedFiles.length > 0) {
            uploadAccountDocuments({ variables: { account: props.accountName, files: acceptedFiles } })
        }
    }, [acceptedFiles, props.accountName]);

    const files = acceptedFiles.map((file: FileWithPath) => (
        <li key={file.path}>
            {file.path} - {file.size} bytes
        </li>
    ));

    return (
        <section className="container">
            <div {...getRootProps({ className: 'dropzone' })}>
                <input {...getInputProps()} />
                <p>Drag 'n' drop some files here, or click to select files</p>
            </div>
            <aside>
                <h4>Files</h4>
                <ul>{files}</ul>
            </aside>
        </section>
    );
}