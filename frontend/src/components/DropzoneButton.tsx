
import { Text, createStyles } from '@mantine/core';
import { Dropzone } from '@mantine/dropzone';

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

export function DropzoneButton() {
  const { classes } = useStyles();

  return (
    <Dropzone onDrop={() => {}} className={classes.dropzone} radius="sm" maxSize={30 * 1024 ** 2}>
      <div style={{ pointerEvents: 'none' }}>
        <Text align="center" weight={700} size="lg" mt="xl">
          <Dropzone.Accept>Drop files here</Dropzone.Accept>
          <Dropzone.Reject>Pdf file less than 30mb</Dropzone.Reject>
          <Dropzone.Idle>Upload resume</Dropzone.Idle>
        </Text>
        <Text align="center" size="sm" mt="xs" color="dimmed">
          Drag&apos;n&apos;drop files here to upload. We can accept only <i>.pdf</i> files that are less than 30mb in size.
        </Text>
      </div>
    </Dropzone>
  );
}
