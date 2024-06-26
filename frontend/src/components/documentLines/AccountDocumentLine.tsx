import { Box, Card, Text } from '@mantine/core';
import { Buffer } from 'buffer';
import { serverBaseUrl } from '../../index';
import { Document } from '../../rest-model';
import { createStyles } from '@mantine/emotion';
import { isDocumentAnImage } from '../../utils/documents';

const useStyles = createStyles((theme, _, u) => ({
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

export interface Props extends Document {
  onClick: (path: string) => void;
}

export default function AccountDocumentLine(props: Props) {
  const { classes } = useStyles();

  const canPreview = isDocumentAnImage(props.path);

  return (
    <Card shadow="sm" p="xs" radius="sm" withBorder onClick={isDocumentAnImage(props.path) ? () => props.onClick(props.path) : undefined}>
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

      <Text fw={500} lineClamp={1} className={classes.title}>
        {props.filename}
      </Text>
    </Card>
  );
}
