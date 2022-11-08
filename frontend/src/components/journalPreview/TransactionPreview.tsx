import { Box, Text, Badge, Group } from '@mantine/core';
import { format } from 'date-fns';
import { TransactionDto } from '../../gql/jouralList';
import Amount from '../Amount';
import { DropzoneButton } from '../DropzoneButton';
import { UPLOAD_TRANSACTION_DOCUMENT } from '../../gql/uploadTransactionDocument';
import Section from '../Section';
import DocumentPreview from './DocumentPreview';
import DashLine from '../DashedLine';

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
        <Group mx={1} my={2} spacing="sm">
          {props.data.links.map((link) => (
            <Badge key={link} size="lg" variant="dot">
              {link}
            </Badge>
          ))}
          {props.data.tags.map((tag) => (
            <Badge key={tag} size="lg" color="orange" variant="dot">
              {tag}
            </Badge>
          ))}
        </Group>
      </Box>

      <Box mx={1} my={4}>
        <Section title="Postings">
          <>
            {props.data.postings.map((posting, idx) => (
              <DashLine key={idx}>
                <Text lineClamp={1} my="xs">
                  {posting.account.name}
                </Text>
                <Text lineClamp={1}>{posting.unit && <Amount amount={posting.unit?.number} currency={posting.unit?.currency} />}</Text>
              </DashLine>
            ))}
          </>
        </Section>
      </Box>

      {props.data.metas.filter((meta) => meta.key !== 'document').length > 0 && (
        <Section title="Metas">
          {props.data.metas
            .filter((meta) => meta.key !== 'document')
            .map((meta, idx) => (
              <DashLine key={idx}>
                <Text lineClamp={1} my="xs">
                  {meta.key}
                </Text>
                <Text lineClamp={1}>{meta.value}</Text>
              </DashLine>
            ))}
        </Section>
      )}

      <Box mx={1} my={4}>
        <Section title={`${props.data.metas.filter((meta) => meta.key === 'document').length} Documents`}>
          <DropzoneButton gql={UPLOAD_TRANSACTION_DOCUMENT} variables={{ file: props.data.spanFile, at: props.data.spanEnd }} />
          <Box>
            {props.data.metas
              .filter((meta) => meta.key === 'document')
              .map((meta, idx) => (
                <DocumentPreview key={idx} uri={meta.value} filename={meta.value} />
              ))}
          </Box>
        </Section>
      </Box>
    </div>
  );
}
