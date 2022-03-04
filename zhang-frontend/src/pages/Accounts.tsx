import AccountLine from "@/components/AccountLine";
import { gql, useQuery } from "@apollo/client";
import { Heading } from "@chakra-ui/react";
import AccountTrie from "../utils/AccountTrie";

export default function Accounts() {
    const { loading, error, data } = useQuery(gql`
    query {
        accounts {
          name
          status
          currencies {
            name
          }
          snapshot {
            inner
          }
        }
      }    
`);

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;

    let trie = new AccountTrie();
    trie.insert({ name: "Assets" });
    for (let account of data.accounts) {
        trie.insert(account);
    }
    console.log("trie", trie);
    return (

        <div>
            {Object.keys(trie.children).sort().map(group => (
                <div>
                    <Heading>{group}</Heading>
                    <div>
                        {Object.keys(trie.children[group].children).map(item => (
                            <AccountLine data={trie.children[group].children[item]} />
                        ))}
                    </div>

                </div>

            ))}

        </div>
    )
}

