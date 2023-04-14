import { Box, createStyles } from '@mantine/core';
import { openContextModal } from '@mantine/modals';
import { Buffer } from 'buffer';
import { serverBaseUrl } from '../..';
import { EXTENSIONS_SUPPORT_PREVIEW } from '../documentLines/AccountDocumentLine';

const useStyles = createStyles((theme, _params, getRef) => ({
  imgBox: {
    overflow: "hidden",
    position: "relative",
    borderRadius: "4px",
    '&:after': {
      content: '" "',
      display: "block",
      paddingBottom: "100%",
    },
  },
  img: {
    '&:hover': {
      cursor: 'pointer',
    },
    position: "absolute",
    // top: theme.spacing.xs,
    // left: theme.spacing.xs,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: "100%",
    height: "100%",
    objectFit: "cover",
  },

  empty: {
    '&:hover': {
      cursor: 'pointer',
    },
    position: "absolute",
    // top: theme.spacing.xs,
    // left: theme.spacing.xs,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: "100%",
    height: "100%",
    backgroundColor: "#f8f9fa",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    textAlign: "center",
  },

  title: {
    '&:hover': {
      cursor: 'pointer',
    },
    fontSize: theme.fontSizes.sm,
    marginTop: theme.spacing.sm
  },

}));
interface Props {
  uri: string;
  filename: string;
}
export default function DocumentPreview({ filename }: Props) {
  const { classes } = useStyles();
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

    <Box className={classes.imgBox} onClick={openDocumentModal}>
      {canPreview
        ? <img
          className={classes.img}
          alt={filename}
          src={canPreview ? `${serverBaseUrl}/api/documents/${Buffer.from(filename).toString('base64')}` : ""}
        />
        : <Box className={classes.empty}>This document cannot be previewed</Box>
      }
    </Box>

  );
}
