import { gql } from "@apollo/client";
import { Connection } from "../models";


export interface ErrorListQuery {
  errors: Connection<ErrorEntity>
}

export interface ErrorEntity {
    message: string,
    span: Span
}

export interface Span {
    start: number,
    end: number,
    filename?: string,
    content: string
}


export const ERROR_LIST = gql`
query ERROR_LIST($cursor: String) {
  errors(first: 10, after: $cursor) {
    edges {
      node {
        message
        span {
          filename
          start
          end
          content
        }
      }
    }
    pageInfo {
      startCursor
      endCursor
      hasPreviousPage
      hasNextPage
    }
  }
}
`