import AccountLine from "@/components/AccountLine";
import { gql, useQuery } from "@apollo/client";
import { Heading } from "@chakra-ui/react";
import AccountTrie from "../utils/AccountTrie";

export default function Documents() {
    const { loading, error, data } = useQuery(gql`
    query DOCUMENT_LIST {
        documents {
          filename
          __typename
          ... on AccountDocumentDto {
            account {
              name
              status
            }
          }
        }
      }
      
         
`);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;

    return (

        <div>
            <Heading>Documents</Heading>
            {data.documents.map(document => {
                switch (document.__typename) {
                    case "AccountDocumentDto":
                        return (
                            <div>{document.filename}</div>
                        )
                    case "TransactionDocumentDto":
                        return (
                            <div>todo transaction document dto line</div>
                        )
                }
            })}
        </div>
    )
}


