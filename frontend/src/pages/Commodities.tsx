import { useQuery } from "@apollo/client";
import { Badge, Box, Flex, Heading, Table, Tbody, Td, Text, Th, Thead, Tr } from "@chakra-ui/react";
import { format } from "date-fns";
import Amount from "../components/Amount";
import { CommoditiesQuery, CURRENCIES } from "../gql/commodities";
import { useNavigate } from "react-router";

export default function Commodities() {
    const { loading, error, data } = useQuery<CommoditiesQuery>(CURRENCIES);
    let navigate = useNavigate();

    const onCommodityClick = (commodityName: string) => {
        navigate(commodityName)
    }

    if (loading) return <p>Loading...</p>;
    if (error) return <p>Error :(</p>;

    return (

        <div>

            <Flex m={1} mb={3} justifyContent="space-between" alignItems="flex-end">
                <Box><Heading>Commodities</Heading></Box>
            </Flex>


            <Table>
                <Thead>
                    <Tr>
                        <Th>Currency</Th>
                        <Th isNumeric>Balance</Th>
                        <Th isNumeric>Latest Price</Th>
                    </Tr>
                </Thead>
                <Tbody>
                    {data?.currencies.map((currency, idx) => (

                        <Tr key={idx}>
                            <Td >
                                <Flex alignItems="center">
                                    <Text onClick={()=> onCommodityClick(currency.name)} cursor="pointer">{currency.name}</Text>
                                    {currency.isOperatingCurrency && (<Badge ml={3} variant='outline' colorScheme="green" >Operating Currency</Badge>)}
                                </Flex>
                            </Td>
                            <Td isNumeric><Amount amount={currency.balance} currency="" /></Td>
                            <Td isNumeric>{currency.latestPrice && (
                                <Flex display="inline-flex" flexDirection={"column"} alignItems="end">
                                    <Amount amount={currency.latestPrice.amount.number} currency={currency.latestPrice.amount.currency} />
                                    <Text fontSize="xs" color="gray.500" display={"inline"}>{format(new Date(currency.latestPrice.date * 1000), "yyyy-MM-dd")}</Text>
                                </Flex>
                            )}</Td>
                        </Tr>
                    ))}
                </Tbody>
            </Table>


        </div>
    )
}


