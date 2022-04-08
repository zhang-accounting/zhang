import { gql, useMutation, useQuery } from "@apollo/client";
import { Button } from "@chakra-ui/react";
import CodeMirror from '@uiw/react-codemirror';
import { useEffect, useState } from "react";
import { SingleFileEntryQuery, SINGLE_FILE_ENTRY } from "../gql/singleFile";

interface Props {
    name?: string,
    path: string
}

export default function SingleFileEdit({ path }: Props) {
    const { loading, error, data } = useQuery<SingleFileEntryQuery>(SINGLE_FILE_ENTRY, {
        variables: {
            name: path
        }
    });
    const [update, _] = useMutation(gql`
    mutation UPDATE_FILE($path: String, $content: String) {
        updateFile(path: $path, content: $content) 
    }
    `, {
        refetchQueries: ["FILE_LIST", "SINGLE_FILE_ENTRY", "JOURNAL_LIST", "BAR_STATISTIC"]
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
                onChange={(value) => {
                    setContent(value)
                }}
            />
            <Button disabled={data?.entry.content === content} onClick={() => update({ variables: { path, content } })}>Update</Button>
        </div>
    )
}