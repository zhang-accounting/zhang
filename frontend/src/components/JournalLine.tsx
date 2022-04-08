import { BalanceCheckDto, BalancePadDto, TransactionDto } from "../gql/jouralList";
import BalanceCheckLine from "./BalanceCheckLine";
import BalancePadLine from "./BalancePadLine";
import TransactionLine from "./TransactionLine";
interface Props {
    data: TransactionDto | BalanceCheckDto | BalancePadDto
}
export default function JournalLine({ data }: Props) {
    switch (data.type) {
        case "BalanceCheckDto":
          return <BalanceCheckLine data={data} />
        case "BalancePadDto":
          return <BalancePadLine data={data} />
        case "TransactionDto":
          return <TransactionLine data={data} />
      }
    
}