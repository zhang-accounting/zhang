import { Card, Group, Image, Text } from '@mantine/core';
import { Buffer } from 'buffer';
import { Document } from '../../rest-model';
import { serverBaseUrl } from '../../index';
import { openContextModal } from '@mantine/modals';

export interface Props extends Document {}

export const EXTENSIONS_SUPPORT_PREVIEW = ['PNG', 'JPG', 'JPEG', 'GIF'];

export default function AccountDocumentLine(props: Props) {
  const extension = (props.extension ?? '').toUpperCase();

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
      <Card.Section>
        {canPreview ? (
          <Image src={`${serverBaseUrl}/api/documents/${Buffer.from(props.path).toString('base64')}`} height={160} />
        ) : (
          <Text style={{ height: 160 }}>this file cannot be previewed</Text>
        )}
      </Card.Section>

      <Group position="apart" mt="md" mb="xs">
        <Text weight={500} lineClamp={1}>
          {props.filename}
        </Text>
        {/* <Badge color="pink" variant="light">
          {extension}
        </Badge> */}
      </Group>
    </Card>
  );
}
