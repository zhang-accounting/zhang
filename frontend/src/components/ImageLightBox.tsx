import { Buffer } from 'buffer';
import * as React from 'react';
import Lightbox from 'yet-another-react-lightbox';
import { serverBaseUrl } from '../global.ts';

interface Props {
  src?: string;
  onChange: (src: string | undefined) => void;
}

export function ImageLightBox(props: Props) {
  return (
    <Lightbox
      slides={[
        {
          src: props.src ? `${serverBaseUrl}/api/documents/${Buffer.from(props.src).toString('base64')}` : '',
        },
      ]}
      open={props.src !== undefined}
      controller={{ closeOnPullDown: false, closeOnBackdropClick: true }}
      close={() => props.onChange(undefined)}
      render={{
        buttonPrev: () => null,
        buttonNext: () => null,
      }}
    />
  );
}
