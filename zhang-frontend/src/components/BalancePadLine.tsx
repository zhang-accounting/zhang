import { BalancePadDto } from "src/gql/jouralList"

interface Props {
    data: BalancePadDto
}

export default function Component({ }: Props) {
    return (
        <div>
            BalancepadLine
        </div>
    )
}