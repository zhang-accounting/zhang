import {
    Badge, Container, createStyles, Group, Paper, Stack, Text, Title
} from '@mantine/core';



const useStyles = createStyles((theme) => ({
    page: {
        height: "100vh",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
    },
    page2: {
        width: "620px"
    }
}));

const servers = [
    { title: "LocalTestServer", url: "http://localhost:8000", version: "0.1.1" },
    { title: "LocalTestServer", url: "http://localhost:8000", version: "0.1.1" }
]

export function Loader() {
    const { classes } = useStyles()
    return (
        <Container className={classes.page}>
            <div className={classes.page2}>
                <Paper withBorder shadow="md" p={30} mt={30} radius="md" >
                    <Title sx={(theme) => ({ fontFamily: `Greycliff CF, ${theme.fontFamily}`})}>
                        Zhang
                    </Title>

                    <Stack mt={30}>
                        {servers.map(server => <Paper withBorder shadow="md" p={20} mt={3}>
                            <Group position='apart'>
                                <Stack spacing="xs">
                                    <Text weight={500}>{server.title}</Text>
                                    <Text size="sm" color="dimmed">
                                        {server.url}
                                    </Text>
                                </Stack>
                                <Badge color="pink" variant="light">
                                    {server.version}
                                </Badge>
                            </Group>

                        </Paper>)}


                    </Stack>
                </Paper>
            </div>

        </Container>
    );
}