import { useQuery } from '@apollo/client';
import { Badge, Container, Group, Table, Text, Title } from '@mantine/core';
import { format } from 'date-fns';
import { useNavigate } from 'react-router';
import Amount from '../components/Amount';
import { CommoditiesQuery, CURRENCIES } from '../gql/commodities';

export default function Commodities() {
  const { loading, error, data } = useQuery<CommoditiesQuery>(CURRENCIES);
  let navigate = useNavigate();

  const onCommodityClick = (commodityName: string) => {
    navigate(commodityName);
  };

  if (loading) return <p>Loading...</p>;
  if (error) return <p>Error :(</p>;

  return (
    <Container fluid>
      <Title order={2}>Commodities</Title>
      <Table verticalSpacing="xs" highlightOnHover>
        <thead>
          <tr>
            <th>Currency</th>
            <th>Balance</th>
            <th>Latest Price</th>
          </tr>
        </thead>
        <tbody>
          {data?.currencies.map((currency, idx) => (
            <tr key={currency.name}>
              <td>
                <Group>
                  <Text onClick={() => onCommodityClick(currency.name)}>{currency.name}</Text>
                  {currency.isOperatingCurrency && (
                    <Badge ml="xs" size="xs" variant="outline">
                      Operating Currency
                    </Badge>
                  )}
                </Group>
              </td>
              <td>
                <Amount amount={currency.balance} currency="" />
              </td>
              <td>
                {currency.latestPrice && (
                  <>
                    <Amount amount={currency.latestPrice.amount.number} currency={currency.latestPrice.amount.currency} />
                    <Text color="dimmed" size="xs">
                      {format(new Date(currency.latestPrice.date * 1000), 'yyyy-MM-dd')}
                    </Text>
                  </>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </Table>
    </Container>
  );
}
