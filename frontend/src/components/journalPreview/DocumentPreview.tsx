// @ts-ignore
import { ActionIcon, Button, Text } from '@mantine/core';
import { IconFileDownload, IconZoomInArea } from '@tabler/icons';
import DashLine from '../DashedLine';
import { EXTENSIONS_SUPPORT_PREVIEW } from '../documentLines/AccountDocumentLine';
import {openContextModal} from "@mantine/modals";

interface Props {
  uri: string;
  filename: string;
}
export default function DocumentPreview({ filename }: Props) {
  const extension = filename.split('.').pop()?.toUpperCase() || '';
  const simpleFilename = filename.split('/').pop() || '';
  const canPreview = EXTENSIONS_SUPPORT_PREVIEW.includes(extension);

  const openDocumentModal = () => {
      openContextModal({
          modal: 'documentPreviewModal',
          title: simpleFilename,
          size:"lg",
          centered: true,
          innerProps: {
              filename: simpleFilename,
              path: filename,
          },
      })
  }
  return (
    <DashLine>
      <Text lineClamp={1} my="xs">
        {simpleFilename}
      </Text>

      <Button.Group>
        {canPreview && (
          <ActionIcon onClick={openDocumentModal}>
            <IconZoomInArea size={18} />
          </ActionIcon>
        )}
        <ActionIcon>
          <IconFileDownload size={18} />
        </ActionIcon>
      </Button.Group>
    </DashLine>
  );
}
