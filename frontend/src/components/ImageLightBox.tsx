import Lightbox from 'yet-another-react-lightbox';
import 'yet-another-react-lightbox/styles.css';
import { documentUrl } from './documentLines/document-utils';

interface Props {
  /** Ledger document path; the lightbox is open while it is set. */
  src?: string;
  onChange: (src: string | undefined) => void;
}

/** Full-screen preview for an image document. Closes on backdrop click / Esc. */
export function ImageLightBox(props: Props) {
  return (
    <Lightbox
      slides={[{ src: props.src ? documentUrl(props.src) : '', alt: props.src?.split(/[/\\]/).pop() }]}
      open={props.src !== undefined}
      controller={{ closeOnPullDown: true, closeOnBackdropClick: true }}
      close={() => props.onChange(undefined)}
      carousel={{ finite: true }}
      styles={{ container: { backgroundColor: 'color-mix(in oklch, black 88%, transparent)' } }}
      render={{
        buttonPrev: () => null,
        buttonNext: () => null,
      }}
    />
  );
}
