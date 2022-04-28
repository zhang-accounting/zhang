import { Dispatch, SetStateAction } from "react";
import { JournalItem } from "../gql/jouralList";
import BalanceCheckLine from "./BalanceCheckLine";
import BalancePadLine from "./BalancePadLine";
import TransactionLine from "./TransactionLine";
interface Props {
  data: JournalItem,
  setSelectedJournal?: Dispatch<SetStateAction<JournalItem | null>>,
}
export default function JournalLine({ data, setSelectedJournal }: Props) {
  let line = null;
  switch (data.type) {
    case "BalanceCheckDto":
      line = <BalanceCheckLine data={data} />;
      break;
    case "BalancePadDto":
      line = <BalancePadLine data={data} />;
      break;
    case "TransactionDto":
      line = <TransactionLine data={data} />;
      break;
  }
  if(setSelectedJournal) {
    return (
      <div onClick={() => setSelectedJournal(data)}>
        {line}
      </div>
    )
  }else {
    return line;
  }
  
}