import { Puzzle } from 'lucide-react';
import { Badge } from './ui/badge';
import { Item, ItemActions, ItemContent, ItemDescription, ItemMedia, ItemTitle } from './ui/item';

interface Props {
  name: string;
  plugin_type: ('Processor' | 'Mapper')[];
  version: string;
}

/** One installed plugin: name, capability badges and version. */
export default function PluginBox(props: Props) {
  return (
    <Item variant="outline" className="bg-card">
      <ItemMedia variant="icon" className="size-9 rounded-lg bg-muted">
        <Puzzle />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="w-full truncate">{props.name}</ItemTitle>
        <ItemDescription className="flex flex-wrap gap-1">
          {props.plugin_type.map((item) => (
            <Badge key={item} variant="secondary">
              {item}
            </Badge>
          ))}
        </ItemDescription>
      </ItemContent>
      <ItemActions>
        <Badge variant="outline" className="font-mono tabular-nums">
          v{props.version}
        </Badge>
      </ItemActions>
    </Item>
  );
}
