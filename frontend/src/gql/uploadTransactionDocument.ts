import { gql } from '@apollo/client';

export const UPLOAD_TRANSACTION_DOCUMENT = gql`
  mutation UPLOAD_TRANSACTION_DOCUMENT($file: String!, $at: Int, $files: [Upload!]!) {
    uploadTransactionDocument(transactionFile: $file, transactionEndLine: $at, files: $files)
  }
`;
