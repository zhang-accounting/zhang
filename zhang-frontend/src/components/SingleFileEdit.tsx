import { gql, useMutation, useQuery } from "@apollo/client";
import { Button, Flex, Text } from "@chakra-ui/react";
import CodeMirror from '@uiw/react-codemirror';
import { useEffect, useState } from "react";

export default function Component({ name, path }) {
    const { loading, error, data } = useQuery(gql`
    query SINGLE_FILE_ENTRY($name: String) {
        entry(name: $name) {
            name
            content
        }
      }    
`, {
        variables: {
            name: path
        }
    });
    const [update, _] = useMutation(gql`
    mutation UPDATE_FILE($path: String, $content: String) {
        updateFile(path: $path, content: $content) 
    }
    `, {
        refetchQueries: ["FILE_LIST", "SINGLE_FILE_ENTRY", "JOURNAL_LIST"]
    })
    const [content, setContent] = useState("");
    useEffect(() => {
        setContent(data?.entry?.content || "");
    }, [data, loading]);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;


    

    return (
        <div>
            <CodeMirror
                value={content}
                height="500px"
                width="100%"
                onChange={(value, viewUpdate) => {
                    setContent(value)
                }}
            />
            <Button disabled={data.entry.content === content} onClick={() => update({ variables: { path, content } })}>Update</Button>
        </div>
    )
}