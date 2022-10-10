import { gql } from '@apollo/client';
import { FileEntry } from '../models';

export interface SingleFileEntryQuery {
  entry: FileEntry;
}

export const SINGLE_FILE_ENTRY = gql`
  query SINGLE_FILE_ENTRY($name: String) {
    entry(name: $name) {
      name
      content
    }
  }
`;
