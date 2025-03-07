import { cn } from '@/lib/utils';
import { useLocalStorage } from '@mantine/hooks';
import { ChevronDownIcon, ChevronRightIcon } from 'lucide-react';
import { useNavigate } from 'react-router';
import AccountTrie from '../utils/AccountTrie';
import Amount from './Amount';
import { Badge } from './ui/badge';
import { TableCell, TableRow } from './ui/table';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './ui/tooltip';

interface Props {
  data: AccountTrie;
  spacing: number;
}

export default function AccountLine({ data, spacing }: Props) {
  let navigate = useNavigate();
  const [isShow, setCollapse] = useLocalStorage({
    key: `account-collapse-${data.path}`,
    defaultValue: false,
  });

  const haveMultipleCommodity = Object.keys(data.amount.data).length > 1;
  const onNavigate = () => {
    if (data?.val?.name) {
      navigate(data?.val?.name);
    } else {
      setCollapse(!isShow);
    }
  };
  const hasChildren = Object.keys(data.children).length > 0;

  return (
    <>
      <TableRow>
        <TableCell>
          <div className="flex items-center gap-2">
            <div style={{ width: `${spacing * 20}px` }}></div>
            {hasChildren ? (
              isShow ? (
                <ChevronDownIcon onClick={() => setCollapse(!isShow)} className="h-4 w-4 cursor-pointer" />
              ) : (
                <ChevronRightIcon onClick={() => setCollapse(!isShow)} className="h-4 w-4 cursor-pointer" />
              )
            ) : (
              <div style={{ width: `20px` }}></div>
            )}
            <div onClick={onNavigate} className="cursor-pointer">
              <div className="flex items-center gap-2">
                <span>{data.val?.alias ?? data.word}</span>
                {data.val?.status === "Close" && <Badge variant="outline">{data.val?.status}</Badge>}
              </div>

              {data.val && <span className="text-xs text-gray-500">{data.val?.name}</span>}
            </div>
          </div>
        </TableCell>
        <TableCell>
          <div className="flex justify-end gap-2">
            {haveMultipleCommodity ? (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger>
                    <div className={cn(data.isLeaf ? 'cursor-pointer' : 'text-gray-500', 'flex gap-2')}>
                      <span>≈</span> <Amount amount={data.amount.total} currency={data.amount.commodity}></Amount>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent>
                    <div className="flex flex-col gap-2">
                      {Object.entries(data.amount.data).map(([key, value]) => (
                        <div className="flex justify-between">
                          <span>+</span>
                          <Amount amount={value} currency={key}></Amount>
                        </div>
                      ))}
                      <div className="flex justify-between">
                        <span>=</span>
                        <Amount amount={data.amount.total} currency={data.amount.commodity}></Amount>
                      </div>
                    </div>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            ) : (
              <div className={cn(data.isLeaf ? ' ' : 'text-gray-500', 'flex gap-2')}>
                <Amount amount={data.amount.total} currency={data.amount.commodity}></Amount>
              </div>
            )}
          </div>
        </TableCell>
      </TableRow>
      {isShow &&
        Object.keys(data.children)
          .sort()
          .map((child) => <AccountLine key={data.children[child].path} data={data.children[child]} spacing={spacing + 1} />)}
    </>
  );
}
