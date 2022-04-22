import { Amount, JournalItem } from "./gql/jouralList"

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