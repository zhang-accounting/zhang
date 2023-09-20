import { Container, Group, Table, Text } from '@mantine/core';
import { format } from 'date-fns';
import { useNavigate } from 'react-router';
import Amount from '../components/Amount';
import { LoadingState } from '../rest-model';
import { useAppSelector } from '../states';
import { Heading } from '../components/basic/Heading';

export default function Commodities() {
  let navigate = useNavigate();

  const onCommodityClick = (commodityName: string) => {
    navigate(commodityName);
  };

  const { value: commodities, status } = useAppSelector((state) => state.commodities);

  if (status === LoadingState.Loading || status === LoadingState.NotReady) return <>loading</>;

  return (
    <Container fluid>
      <Heading title={`Commodities`}></Heading>
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
          {Object.entries(commodities).map(([_, currency]) => (
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
