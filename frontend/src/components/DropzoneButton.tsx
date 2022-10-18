import { useMutation } from '@apollo/client';
import { Text, createStyles } from '@mantine/core';
import { Dropzone, FileWithPath } from '@mantine/dropzone';
import { DocumentNode } from 'graphql';
import { useEffect, useState } from 'react';

const useStyles = createStyles((theme) => ({
  dropzone: {
    borderWidth: 1,
    paddingBottom: 50,
  },

  icon: {
    color: theme.colorScheme === 'dark' ? theme.colors.dark[3] : theme.colors.gray[4],
  },

  control: {
    position: 'absolute',
    width: 250,
    left: 'calc(50% - 125px)',
    bottom: -20,
  },
}));

interface Props {
  gql: DocumentNode;
  variables: any;
}

export function DropzoneButton({ gql, variables }: Props) {
  const { classes } = useStyles();

  const [files, setFiles] = useState<FileWithPath[]>([]);

  const [uploadDocuments] = useMutation(gql, {
    refetchQueries: [],

    update: (proxy) => {
      // todo evict cache
    },
  });

  useEffect(() => {
    console.log('files', files);
    if (files.length > 0) {
      uploadDocuments({
        variables: { ...variables, files: files },
      }).then(() => {
        setTimeout(() => {
          setFiles([]);
        }, 1000);
      });
    }
  }, [files, variables, uploadDocuments]);

  return (
    <Dropzone onDrop={setFiles} className={classes.dropzone} radius="sm" maxSize={30 * 1024 ** 2}>
      <div style={{ pointerEvents: 'none' }}>
        <Text align="center" weight={700} size="lg" mt="xl">
          <Dropzone.Accept>Drop files here</Dropzone.Accept>
          <Dropzone.Reject>Pdf file less than 30mb</Dropzone.Reject>
          <Dropzone.Idle>Upload documents</Dropzone.Idle>
        </Text>
        <Text align="center" size="sm" mt="xs" color="dimmed">
          Drag&apos;n&apos;drop files here to upload.
        </Text>
      </div>
    </Dropzone>
  );
}
