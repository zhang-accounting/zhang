import { Button } from "@/components/ui/button"
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuLabel,
    DropdownMenuSeparator,
    DropdownMenuTrigger,
  } from "@/components/ui/dropdown-menu"
import { Ellipsis } from "lucide-react"

interface Props {
  actions: {
    label: string;
    icon: React.ElementType;
    onClick: () => void;
  }[]
}

  
export function LineMenu(props: Props) {

    return (<DropdownMenu>
        <DropdownMenuTrigger><Button variant="ghost" size="icon"><Ellipsis className="w-4 h-4" /></Button></DropdownMenuTrigger>
        <DropdownMenuContent>
          {props.actions.map((action, index) => (
            <DropdownMenuItem key={index} onClick={action.onClick}>
              <action.icon className="w-4 h-4 mr-2" />
              {action.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>)
}