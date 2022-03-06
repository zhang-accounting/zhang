import SingleFileEdit from "@/components/SingleFileEdit";
import { gql, useQuery } from "@apollo/client";
import { Box, Container, SimpleGrid, Stat, Tab, TabList, TabPanel, TabPanels, Tabs } from "@chakra-ui/react";

function RawEdit() {
    const { loading, error, data } = useQuery(gql`
    query FILE_LIST {
        entries {
            name
        }
      }    
`);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;



    return (
        <Box as="section">
            <Tabs isLazy>
                <TabList>
                    {data.entries.map(entry => (
                        <Tab key={entry.name}>{entry.name.split("/").pop()}</Tab>
                    ))}
                </TabList>

                <TabPanels>
                    {data.entries.map(entry => (
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
