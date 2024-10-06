import { Buffer } from 'buffer';
import { serverBaseUrl } from '../../global.ts';
import { isDocumentAnImage } from '../../utils/documents';

interface Props {
  uri: string;
  filename: string;
  onClick: (path: string) => void;
}

export default function DocumentPreview(props: Props) {
  const canPreview = isDocumentAnImage(props.filename);

  return (
    <div
      className="relative overflow-hidden rounded-md after:content-[''] after:block after:pb-[100%]"
      onClick={canPreview ? () => props.onClick(props.filename) : undefined}
    >
      {canPreview ? (
        <img
          className="absolute top-0 left-0 right-0 bottom-0 w-full h-full object-cover hover:cursor-pointer"
          alt={props.filename}
          src={canPreview ? `${serverBaseUrl}/api/documents/${Buffer.from(props.filename).toString('base64')}` : ''}
        />
      ) : (
        <div className="absolute top-0 left-0 right-0 bottom-0 w-full h-full bg-gray-100 flex items-center justify-center text-center hover:cursor-pointer">
          This document cannot be previewed
        </div>
      )}
    </div>
  );
}
