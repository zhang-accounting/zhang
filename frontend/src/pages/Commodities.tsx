import { useQuery } from '@apollo/client';
import { Badge, Container, Group, Table, Text, Title } from '@mantine/core';
import { format } from 'date-fns';
import { useNavigate } from 'react-router';
import useSWR from 'swr';
import { fetcher } from '..';
import Amount from '../components/Amount';
import { CommoditiesQuery, CURRENCIES } from '../gql/commodities';
import { CommodityListItem } from '../rest-model';

export default function Commodities() {
  const { data, error } = useSWR<CommodityListItem[]>("/api/commodities", fetcher)
  let navigate = useNavigate();

  const onCommodityClick = (commodityName: string) => {
    navigate(commodityName);
  };

  if (error) return <div>failed to load</div>;
  if (!data) return <>loading</>;

  return (
    <Container fluid>
      <Title order={2}>Commodities</Title>
      <Table verticalSpacing="xs" highlightOnHover>
        <thead>
          <tr>
            <th>Currency</th>
            <th>Precision</th>
            <th>Balance</th>
            <th>Latest Price</th>
          </tr>
        </thead>
        <tbody>
          {data.map((currency) => (
            <tr key={currency.name}>
              <td>
                <Group>
                  <Text onClick={() => onCommodityClick(currency.name)}>{currency.name}</Text>
                </Group>
              </td>
              <td>{currency.precision}</td>
              <td>
                <Amount amount={currency.total_amount} currency="" />
              </td>
              <td>
                {currency.latest_price_amount && (
                  <>
                    <Amount amount={currency.latest_price_amount} currency={currency.latest_price_commodity} />
                    <Text color="dimmed" size="xs" align="right">
                      {format(new Date(currency.latest_price_date), 'yyyy-MM-dd')}
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
