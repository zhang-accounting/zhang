import { gql, useQuery } from "@apollo/client";
import { Link } from "react-router-dom";

function Journals() {
  const { loading, error, data } = useQuery(gql`
  query {
    journals {
      date
      type:__typename
    ... on TransactionDto {
      payee
      narration
    }
    }
  }
`);

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;
  console.log(data);


  return (
    <table>
            <thead>

                <tr>
                    <th>date</th>
                    <th>type</th>
                    <th>payee narration</th>
                    <th>#</th>
                </tr>
            </thead>
            <tbody>
                {data.journals.map((journal) => (
                    <tr>
                        <td>
                            {journal.date}
                        </td>
                        <td>
                            {journal.type}
                        </td>
                        <td>
                            {`${journal?.payee} ${journal?.narration}`}
                        </td>
                        <td></td>
                    </tr>
                ))}
            </tbody>
        </table>
  );
}

export default Journals;
