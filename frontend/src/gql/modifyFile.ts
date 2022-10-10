import { gql } from '@apollo/client';

export const MODIFY_FILE = gql`
  mutation ($file: String, $content: String, $start: Int, $end: Int) {
    modifyData(file: $file, content: $content, start: $start, end: $end)
  }
`;
