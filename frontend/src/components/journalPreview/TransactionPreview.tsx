import { Box, Text, Badge, Group } from '@mantine/core';
import { format } from 'date-fns';
import { TransactionDto } from '../../gql/jouralList';
import Amount from '../Amount';
import Block from '../Block';
import { DropzoneButton } from '../DropzoneButton';
import { UPLOAD_TRANSACTION_DOCUMENT } from '../../gql/uploadTransactionDocument';

interface Props {
  data: TransactionDto;
}
export default function TransactionPreview(props: Props) {
  return (
    <div>
      <Box mb={10}>
        <Box mx={1} my={2}>
          {format(new Date(props.data.timestamp * 1000), 'yyyy-MM-dd hh:mm:ss')}
        </Box>
        {!props.data.isBalanced && (
          <Box mx={1} my={2}>
            <Text color={'red'}>UNBALANCED</Text>
          </Box>
        )}
        <Box mx={1} my={2}>
          <Text>{props.data.payee}</Text>
        </Box>
        <Box mx={1} my={2}>
          {props.data.narration}
        </Box>
        <Box mx={1} my={2}>
          {props.data.tags.map((tag) => (
            <Badge variant="outline">#{tag}</Badge>
          ))}
          {props.data.links.map((link) => (
            <Badge variant="outline">^{link}</Badge>
          ))}
        </Box>
      </Box>

      <Box mx={1} my={4}>
        <Block title="Postings">
          <>
            {props.data.postings.map((posting, idx) => (
              <Group key={idx} position="apart">
                <div>{posting.account.name}</div>
                <div>{posting.unit && <Amount amount={posting.unit?.number} currency={posting.unit?.currency} />}</div>
              </Group>
            ))}
          </>
        </Block>
      </Box>
      {props.data.metas.filter((meta) => meta.key !== 'document').length > 0 && (
        <Box mx={1} my={4}>
          <Block title="Metas">
            <Box>
              {props.data.metas
                .filter((meta) => meta.key !== 'document')
                .map((meta, idx) => (
                  <Group key={idx} position="apart">
                    <div>{meta.key}</div>
                    <div>{meta.value}</div>
                  </Group>
                ))}
            </Box>
          </Block>
        </Box>
      )}

      <Box mx={1} my={4}>
        <Block title={`${props.data.metas.filter((meta) => meta.key === 'document').length} Documents`}>
          <DropzoneButton gql={UPLOAD_TRANSACTION_DOCUMENT} variables={{ file: props.data.spanFile, at: props.data.spanEnd }} />
          <Box>
            {props.data.metas
              .filter((meta) => meta.key === 'document')
              .map((meta, idx) => (
                <Group key={idx} position="apart">
                  <div>{meta.value}</div>
                </Group>
              ))}
          </Box>
        </Block>
      </Box>
    </div>
  );
}
