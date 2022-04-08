import { gql } from "@apollo/client";



export const UPLOAD_ACCOUNT_DOCUMENT = gql`
   mutation UPLOAD_ACCOUNT_DOCUMENT($account: String!, $files:[Upload!]!) {
    uploadAccountDocument(accountName:$account, files: $files)
   } 
`