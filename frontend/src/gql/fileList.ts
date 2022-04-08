import { gql } from "@apollo/client";
import { FileEntry } from "../models";


export interface FileListQuery {
    entries: FileEntry[]
}


export const FILE_LIST = gql`
query FILE_LIST {
    entries {
        name
    }
  }    
`