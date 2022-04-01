import { Amount } from "./gql/jouralList"

export interface Snapshot {
    summary: Amount,
    detail: Amount[]
}
export interface FileEntry {
    name: string,
    content: string
}