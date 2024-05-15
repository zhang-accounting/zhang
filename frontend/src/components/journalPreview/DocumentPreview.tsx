import { Box } from '@mantine/core';
import { Buffer } from 'buffer';
import { serverBaseUrl } from '../..';
import { createStyles } from '@mantine/emotion';
import { isDocumentAnImage } from '../../utils/documents';

const useStyles = createStyles((theme, _, u) => ({
  imgBox: {
    overflow: 'hidden',
    position: 'relative',
    borderRadius: '4px',
    '&:after': {
      content: '" "',
      display: 'block',
      paddingBottom: '100%',
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

interface Props {
  uri: string;
  filename: string;
  onClick: (path: string) => void;
}

export default function DocumentPreview(props: Props) {
  const { classes } = useStyles();
  const canPreview = isDocumentAnImage(props.filename);

  return (
    <Box className={classes.imgBox} onClick={canPreview ? () => props.onClick(props.filename) : undefined}>
      {canPreview ? (
        <img
          className={classes.img}
          alt={props.filename}
          src={canPreview ? `${serverBaseUrl}/api/documents/${Buffer.from(props.filename).toString('base64')}` : ''}
        />
      ) : (
        <Box className={classes.empty}>This document cannot be previewed</Box>
      )}
    </Box>
  );
}
