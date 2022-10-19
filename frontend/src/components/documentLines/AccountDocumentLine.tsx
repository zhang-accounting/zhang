import { Text, Image, Card, Group } from '@mantine/core';
import { Buffer } from 'buffer';
import { AccountItem } from '../../gql/accountList';
import { TransactionDto } from '../../gql/jouralList';

export interface DocumentRenderItem {
  filename: string;
  accounts: (AccountItem | undefined)[];
  transactions: (TransactionDto | undefined)[];
}

export interface Props extends DocumentRenderItem {}

export const EXTENSIONS_SUPPORT_PREVIEW = ['PNG', 'JPG', 'GIF'];

export default function AccountDocumentLine({ filename }: Props) {
  const extension = filename.split('.').pop()?.toUpperCase() || '';
  const simpleFilename = filename.split('/').pop() || '';
  const canPreview = EXTENSIONS_SUPPORT_PREVIEW.includes(extension);
  return (
    <Card shadow="sm" p="xs" radius="sm" withBorder>
      <Card.Section>
        {canPreview ? (
          <Image src={`/files/${Buffer.from(filename).toString('base64')}/preview`} height={160} />
        ) : (
          <Text style={{ height: 160 }}>this file cannot be previewed</Text>
        )}
      </Card.Section>

      <Group position="apart" mt="md" mb="xs">
        <Text weight={500} lineClamp={1}>
          {simpleFilename}
        </Text>
        {/* <Badge color="pink" variant="light">
          {extension}
        </Badge> */}
      </Group>
    </Card>
  );
}
