import { Check, ChevronsUpDown } from "lucide-react"
import { Popover, PopoverContent, PopoverTrigger } from "./popover"
import { Button } from "./button"
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "./command"
import React from "react"
import { cn } from "@/lib/utils"

interface Props {

  value?: string
  onChange?: (value?: string) => void
  options: {
    group: string
    items: {
      value: string
      label: string
    }[]
  }[]
}

export function Combobox(props: Props) {
  const [open, setOpen] = React.useState(false)
  console.log("combobox", props.value);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          role="combobox"
          aria-expanded={open}
          className="w-full justify-between"
        >
          {props.value
            ? props.options.flatMap(group => group.items).find(item => item.value === props.value)?.label
            : "Select Account..."}
          <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-full p-0">
        <Command>
          <CommandInput placeholder="input to search..." />
          <CommandList>
            <CommandEmpty>No item found.</CommandEmpty>
            {props.options.map((group) => (
              <CommandGroup key={group.group} heading={group.group}>
                {group.items.map((option) => (
                  <CommandItem
                    key={option.value}
                    value={option.value}
                    onSelect={(currentValue) => {
                      setOpen(false);
                      props.onChange?.(currentValue);
                    }}
                  >
                    <Check
                    className={cn(
                      "mr-2 h-4 w-4",
                      props.value === option.value ? "opacity-100" : "opacity-0"
                    )}
                  />
                    {option.label}
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}