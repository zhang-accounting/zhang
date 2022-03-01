import { gql, useQuery } from "@apollo/client";

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

    return (
        <table>
            <thead>

                <tr>
                    <th>status</th>
                    <th>account name</th>
                    <th>currencies</th>
                    <th>balances</th>
                </tr>
            </thead>
            <tbody>
                {data.accounts.map((account) => (
                    <tr key={account.name}>
                        <td>
                            {account.status}
                        </td>
                        <td>
                            {account.name}
                        </td>
                        <td>
                            {account.currencies.map(currency => <span key={currency.name}>{currency.name}</span>)}
                        </td>
                        <td>
                            {Object.keys(account.snapshot.inner).map(currency => <span>{account.snapshot.inner[currency]} {currency}</span>)}
                        </td>
                    </tr>
                ))}
            </tbody>
        </table>
    )
}


