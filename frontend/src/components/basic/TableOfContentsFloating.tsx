import { Button } from '../ui/button';
import { DropdownMenu, DropdownMenuTrigger } from '@radix-ui/react-dropdown-menu';
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from '../ui/dropdown-menu';

export const ZHANG_VALUE = Symbol();

export interface Tier {
  [ZHANG_VALUE]?: string;

  [key: string]: Tier;
}

interface TableOfContentsFloatingProps {
  files: Tier;

  onChange(value: string): void;
}

export function TableOfContentsFloating({ files, onChange }: TableOfContentsFloatingProps) {
  const itemBuilder = (item: Tier, index: number) => {
    return Object.keys(item).map((key) => {
      const targetItem = item[key];
      const isFile = !!targetItem[ZHANG_VALUE];
      return (
        <>
          {isFile ? (
            <DropdownMenuItem onClick={() => onChange(targetItem[ZHANG_VALUE]!)}>{key}</DropdownMenuItem>
          ) : (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>{key}</DropdownMenuSubTrigger>
              <DropdownMenuPortal>
                <DropdownMenuSubContent>{itemBuilder(targetItem, index + 1)}</DropdownMenuSubContent>
              </DropdownMenuPortal>
            </DropdownMenuSub>
          )}
        </>
      );
    });
  };
  const items = itemBuilder(files, 1);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="outline">Open</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent className="w-56">{items}</DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}
