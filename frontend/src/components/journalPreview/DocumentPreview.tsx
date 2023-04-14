import { createStyles, Image, Text } from '@mantine/core';
import { openContextModal } from '@mantine/modals';
import { Buffer } from 'buffer';
import { serverBaseUrl } from '../..';
import { EXTENSIONS_SUPPORT_PREVIEW } from '../documentLines/AccountDocumentLine';

const useStyles = createStyles((theme, _params, getRef) => ({
  img: {
    '&:hover': {
      cursor: 'pointer',
    },
  },

}));
interface Props {
  uri: string;
  filename: string;
}
export default function DocumentPreview({ filename }: Props) {
  const {classes} = useStyles();
  const extension = filename.split('.').pop()?.toUpperCase() || '';
  const simpleFilename = filename.split('/').pop() || '';
  const canPreview = EXTENSIONS_SUPPORT_PREVIEW.includes(extension);

  const openDocumentModal = () => {
    openContextModal({
      modal: 'documentPreviewModal',
      title: simpleFilename,
      size: 'lg',
      centered: true,
      innerProps: {
        filename: simpleFilename,
        path: filename,
      },
    });
  };
  return (
    <Image
      className={classes.img}
      height={"121px"}
      width={"121px"}
      radius={"sm"}
      onClick={openDocumentModal}
      src={canPreview ? `${serverBaseUrl}/api/documents/${Buffer.from(filename).toString('base64')}`: ""}
      fit='cover'
      alt="With custom placeholder"
      withPlaceholder
      placeholder={<Text align="center">This document cannot be previewed</Text>}
    />
  );
}
