import { Badge, Box, Container, Grid, Paper, Text, Image } from '@mantine/core';
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
    <Box style={{ width: '200px', height: '200px' }}>
      <Grid>
        <Grid.Col span="content">
          <Badge variant="outline">{extension}</Badge>
        </Grid.Col>
        <Grid.Col span={6}>
          <Text mx={2}>{simpleFilename}</Text>
        </Grid.Col>
      </Grid>
      <Box>{canPreview ? <Image src={`/files/${Buffer.from(filename).toString('base64')}/preview`} /> : <Text>this file cannot be previewed</Text>}</Box>
    </Box>
  );
}
