import { Amount, JournalItem } from "./gql/jouralList"

export interface Connection<T> {
    pageInfo: Pagination,
    edges: Edge<T>[]
}

export interface Pagination {
    hasPreviousPage: boolean,
    hasNextPage: boolean,
    startCursor: string,
    endCursor: string
}

export interface Edge<T> {
    node: T,
    cursor: string
}


export interface Snapshot {
    summary: Amount,
    detail: Amount[],
}
export interface FileEntry {
    name: string,
    content: string
}

export type Statistic = {
    start: number,
    end: number,
    frames: Statistic[],
    journals: JournalItem[]

} & {
    [key: string]: Snapshot
}