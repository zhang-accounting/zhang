

export interface Connection<T> {
  pageInfo: Pagination;
  edges: Edge<T>[];
}

export interface Pagination {
  hasPreviousPage: boolean;
  hasNextPage: boolean;
  startCursor: string;
  endCursor: string;
}

export interface Edge<T> {
  node: T;
  cursor: string;
}

export interface FileEntry {
  name: string;
  content: string;
}
