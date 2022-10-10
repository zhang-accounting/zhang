import { gql } from '@apollo/client';
import { Document } from './accountList';

export interface DocumentListQuery {
  documents: Document[];
}

export const DOCUMENT_LIST = gql`
  query DOCUMENT_LIST {
    documents {
      date
      filename
      __typename
      ... on AccountDocumentDto {
        account {
          name
          status
        }
      }
      ... on TransactionDocumentDto {
        transaction {
          payee
          narration
        }
      }
    }
  }
`;
