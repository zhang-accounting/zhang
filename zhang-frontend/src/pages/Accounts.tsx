import AccountLine from "@/components/AccountLine";
import { gql, useQuery } from "@apollo/client";
import { Checkbox, Heading, useBoolean } from "@chakra-ui/react";
import { useEffect, useState } from "react";
import AccountTrie from "../utils/AccountTrie";

export default function Accounts() {
  const { loading, error, data } = useQuery(gql`
    query {
        accounts {
          name
          status
          snapshot {
            summary{
              number
              currency
            }
          }
          currencies {
            name
          }
        }
      }
         
`);

  const [hideClosedAccount, sethideClosedAccount] = useBoolean(false);
  const [accountTrie, setAccountTrie] = useState(new AccountTrie());

  useEffect(() => {
    if (data) {
      let trie = new AccountTrie();
      trie.insert({ name: "Assets" });
      console.log(data.accounts.filter(it => hideClosedAccount ? it.status === "OPEN" : true));
      for (let account of data.accounts.filter(it => hideClosedAccount ? it.status === "OPEN" : true)) {
        trie.insert(account);
      }
      setAccountTrie(trie);
    }
  }, [data, hideClosedAccount]);


  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  return (

    <div>
      <Heading>Accounts </Heading>
      <Checkbox checked={hideClosedAccount} onChange={sethideClosedAccount.toggle}>Hide Closed Account</Checkbox>
      {Object.keys(accountTrie.children).sort().map(group => (
        <div key={group}>
          <Heading size={"l"}>{group}</Heading>
          <div>
            {Object.keys(accountTrie.children[group].children).sort().map(item => (
              <AccountLine data={accountTrie.children[group].children[item]} />
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}


