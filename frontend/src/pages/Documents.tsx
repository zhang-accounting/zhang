import AccountDocumentLine from "../components/documentLines/AccountDocumentLine";
import { useQuery } from "@apollo/client";
import { Heading } from "@chakra-ui/react";
import { DocumentListQuery, DOCUMENT_LIST } from "../gql/documentList";
import TransactionDocumentLine from "../components/documentLines/TransactionDocumentLine";

export default function Documents() {
    const { loading, error, data } = useQuery<DocumentListQuery>(DOCUMENT_LIST);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;

    return (

        <div>
            <Heading>{data?.documents.length} Documents</Heading>
            {data?.documents.map(document => {
                switch (document.__typename) {
                    case "AccountDocumentDto":
                        return (
                            <AccountDocumentLine {...document} />
                        )
                    case "TransactionDocumentDto":
                        return (
                            <TransactionDocumentLine {...document} />
                        )
                    default:
                        return(<div></div>);
                }
            })}
        </div>
    )
}


