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
  single?: boolean;
  onResult?(result: any): void;
}

export function DropzoneButton({ gql, variables, single, onResult }: Props) {
  const singleFlag = single || false;
  const { classes } = useStyles();

  const [files, setFiles] = useState<FileWithPath[]>([]);

  const [uploadDocuments] = useMutation(gql, {
    refetchQueries: [],

    update: (proxy) => {
      // todo evict cache
    },
  });

  useEffect(() => {
    if (files.length > 0) {
      let fileVariable;
      if (singleFlag) {
        fileVariable = { file: files[0] };
      } else {
        fileVariable = { files };
      }
      uploadDocuments({
        variables: { ...variables, ...fileVariable },
      }).then((result) => {
        if (onResult) {
          onResult(result);
        }
        setTimeout(() => {
          setFiles([]);
        }, 1000);
      });
    }
  }, [files, variables, uploadDocuments, singleFlag, onResult]);

  return (
    <Dropzone onDrop={setFiles} className={classes.dropzone} radius="sm" maxSize={30 * 1024 ** 2} multiple={!singleFlag}>
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
