import {JournalBalanceCheckItem} from "../../../rest-model";
import {Badge, createStyles} from "@mantine/core";
import {format} from "date-fns";
import Amount from "../../Amount";
import BigNumber from "bignumber.js";

const useStyles = createStyles((theme) => ({
    payee: {
        fontWeight: "bold",
    },
    narration: {},
    positiveAmount: {
        color: theme.colors.gray[7],
        fontWeight: 'bold',
        fontFeatureSettings: 'tnum',
        fontSize: theme.fontSizes.sm * 0.95,
    },
    negativeAmount: {
        color: theme.colors.red[5],
        fontWeight: 'bold',
        fontFeatureSettings: 'tnum',
        fontSize: theme.fontSizes.sm,
    },
    notBalance: {
        borderLeft: "3px solid red"
    },
    wrapper: {
        display: "flex",
        flexDirection: "column",
        alignItems: "end",
    }
}));

interface Props {
    data: JournalBalanceCheckItem
}

export default function TableViewBalanceCheckLine({data}: Props) {
    const {classes} = useStyles();

    const date = format(new Date(data.datetime), 'yyyy-MM-dd');
    const time = format(new Date(data.datetime), 'hh:mm:ss');


    const isBalanced = new BigNumber(data.postings[0].account_after_number).eq(new BigNumber(data.postings[0].account_before_number))
    return (
        <tr className={!isBalanced ? classes.notBalance : ""}>
            <td>{date} {time}</td>
            <td><Badge size="xs" variant="outline">Check</Badge></td>
            <td>{data.payee}</td>
            <td>{data.narration}</td>
            <td>
                <div className={classes.wrapper}>
                    <div className={!isBalanced ? classes.negativeAmount : classes.positiveAmount}>
                        <Amount amount={data.postings[0].account_after_number}
                                currency={data.postings[0].account_after_commodity}/>
                    </div>
                    {!isBalanced &&
                        <span className={classes.positiveAmount}>
                            current: <Amount amount={data.postings[0].account_before_number}
                                             currency={data.postings[0].account_before_commodity}/>
                        </span>
                    }
                </div>

            </td>
            <td></td>
        </tr>
    );
}
