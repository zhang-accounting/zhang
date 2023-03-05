import {Text, Group, createStyles} from '@mantine/core';
import {useAtom} from 'jotai';
import {commoditiesAtom} from "../states/commodity";

const useStyles = createStyles((theme) => ({
    number: {
        fontFeatureSettings: '"tnum" 1',
    },
    postfix: {
        fontFeatureSettings: '"tnum" 1',
    },
}));

interface Props {
    amount: string | number;
    currency: string;
    negetive?: boolean;
}

export default function Amount({amount, currency, negetive}: Props) {
    const {classes} = useStyles();
    const [commodities] = useAtom(commoditiesAtom);
    const commodity = commodities[currency];

    const flag = negetive || false ? -1 : 1;

    var formatter = new Intl.NumberFormat('en-US', {
        minimumFractionDigits: commodity?.precision ?? 2,
        maximumFractionDigits: commodity?.precision ?? 10,
    });
    const parsedValue =  typeof amount === 'string' ? parseFloat(amount) : amount;
    const value = parsedValue === 0 ? parsedValue : flag * parsedValue;
    const shouldDisplayCurrencyName = !!!commodity?.prefix && !!!commodity?.suffix;

    return (
        <Group spacing={'xs'} position="right">
            {commodity?.prefix &&
                <Text mx={1} className={classes.postfix}>
                    {commodity?.prefix}
                </Text>
            }
            <Text className={classes.number}>{formatter.format(value)}</Text>
            {commodity?.suffix &&
                <Text mx={1} className={classes.postfix}>
                    {commodity?.suffix}
                </Text>
            }
            {shouldDisplayCurrencyName &&
                <Text mx={1} className={classes.postfix}>
                    {currency}
                </Text>
            }
        </Group>
    );
}
