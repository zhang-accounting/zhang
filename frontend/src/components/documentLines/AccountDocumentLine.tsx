import { Box, Card, createStyles, Text } from '@mantine/core';
import { openContextModal } from '@mantine/modals';
import { Buffer } from 'buffer';
import { serverBaseUrl } from '../../index';
import { Document } from '../../rest-model';

const useStyles = createStyles((theme, _params, getRef) => ({
  imgBox: {
    overflow: 'hidden',
    position: 'relative',
    '&:after': {
      content: '" "',
      display: 'block',
      paddingBottom: '75%',
    },
  },
  img: {
    '&:hover': {
      cursor: 'pointer',
    },
    position: 'absolute',
    // top: theme.spacing.xs,
    // left: theme.spacing.xs,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: '100%',
    height: '100%',
    objectFit: 'cover',
  },

  empty: {
    '&:hover': {
      cursor: 'pointer',
    },
    position: 'absolute',
    // top: theme.spacing.xs,
    // left: theme.spacing.xs,
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
    width: '100%',
    height: '100%',
    backgroundColor: '#f8f9fa',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    textAlign: 'center',
  },

  title: {
    '&:hover': {
      cursor: 'pointer',
    },
    fontSize: theme.fontSizes.sm,
    marginTop: theme.spacing.sm,
  },
}));

export interface Props extends Document {}

export const EXTENSIONS_SUPPORT_PREVIEW = ['image/png', 'image/jpg', 'image/jpeg', 'image/gif'];

export default function AccountDocumentLine(props: Props) {
  const { classes } = useStyles();

  const extension = (props.extension ?? '').toLowerCase();
  console.log(props, extension);
  const canPreview = EXTENSIONS_SUPPORT_PREVIEW.includes(extension);
  const openPreviewModal = () => {
    if (canPreview) {
      openContextModal({
        modal: 'documentPreviewModal',
        title: props.filename,
        size: 'lg',
        centered: true,
        innerProps: {
          filename: props.filename,
          path: props.path,
        },
      });
    }
  };
  return (
    <Card shadow="sm" p="xs" radius="sm" withBorder onClick={openPreviewModal}>
      <Card.Section className={classes.imgBox}>
        {canPreview ? (
          <img
            className={classes.img}
            alt={props.filename}
            src={canPreview ? `${serverBaseUrl}/api/documents/${Buffer.from(props.path).toString('base64')}` : ''}
          />
        ) : (
          <Box className={classes.empty}>This document cannot be previewed</Box>
        )}
      </Card.Section>

      <Text weight={500} lineClamp={1} className={classes.title}>
        {props.filename}
      </Text>
    </Card>
  );
}
