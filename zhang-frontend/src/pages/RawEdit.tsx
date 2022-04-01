import SingleFileEdit from "@/components/SingleFileEdit";
import { useQuery } from "@apollo/client";
import { Box, Tab, TabList, TabPanel, TabPanels, Tabs } from "@chakra-ui/react";
import { FileListQuery, FILE_LIST } from "../gql/fileList";

function RawEdit() {
    const { loading, error, data } = useQuery<FileListQuery>(FILE_LIST);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;



    return (
        <Box as="section">
            <Tabs isLazy>
                <TabList>
                    {data?.entries.map(entry => (
                        <Tab key={entry.name}>{entry.name.split("/").pop()}</Tab>
                    ))}
                </TabList>

                <TabPanels>
                    {data?.entries.map(entry => (
                        <TabPanel key={entry.name}>
                            <SingleFileEdit name={entry.name.split("/").pop()} path={entry.name} />
                        </TabPanel>
                    ))}
                </TabPanels>
            </Tabs>
        </Box>
    )
}

export default RawEdit;
