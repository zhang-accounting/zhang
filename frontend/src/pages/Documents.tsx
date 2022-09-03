import { useQuery } from "@apollo/client";
import { Box, Flex, Heading } from "@chakra-ui/react";
import AccountDocumentLine, { DocumentRenderItem } from "../components/documentLines/AccountDocumentLine";
import { DocumentListQuery, DOCUMENT_LIST } from "../gql/documentList";

export default function Documents() {
    const { loading, error, data } = useQuery<DocumentListQuery>(DOCUMENT_LIST);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;
    const documents: { [filename: string]: DocumentRenderItem } = {};

    for (let document of data!.documents) {

        let filename = document.filename;
        if (!documents.hasOwnProperty(filename)) {
            documents[filename] = {
                filename: filename,
                accounts: [],
                transactions: []
            } as DocumentRenderItem
        }

        switch (document.__typename) {
            case "AccountDocumentDto":
                documents[filename].accounts.push(document.account);
                break;
            case "TransactionDocumentDto":
                documents[filename].transactions.push(document.transaction);
                break;
            default:

        }
    }
    return (

        <Box h="calc(100vh - var(--chakra-sizes-20))" maxH="calc(100vh - var(--chakra-sizes-20))" overflow="scroll">
            <Heading>{Object.keys(documents).length} Documents</Heading>
            <Flex flexFlow={"row wrap"} alignContent="flex-start">
                {Object.values(documents).map((document, idx) =>
                    <AccountDocumentLine key={idx} {...document} />
                )}
            </Flex>

        </Box>
    )
}


