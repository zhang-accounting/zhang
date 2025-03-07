import { Buffer } from 'buffer';
import { isDocumentAnImage } from '../../utils/documents';
import { Document } from '@/api/types';

export interface Props extends Document {
  onClick: (path: string) => void;
}

export default function AccountDocumentLine(props: Props) {
  const canPreview = isDocumentAnImage(props.path);

  return (
    <div className="flex flex-col border rounded-sm shadow-sm" onClick={isDocumentAnImage(props.path) ? () => props.onClick(props.path) : undefined}>
      <div className="relative after:content-[''] after:block after:pb-[75%]">
        {canPreview ? (
          <img
            className="absolute inset-0 w-full h-full object-cover hover:cursor-pointer rounded-t-sm"
            alt={props.filename}
            src={canPreview ? `/api/documents/${Buffer.from(props.path).toString('base64')}` : ''}
          />
        ) : (
          <p className="absolute inset-0 w-full h-full flex items-center justify-center text-center bg-gray-100 text-gray-500">
            This document cannot be previewed
          </p>
        )}
      </div>
      <div className="text-sm m-2 font-medium truncate hover:cursor-pointer">
        {props.filename}
        {/* todo add tooltips */}
      </div>
    </div>
  );
}
