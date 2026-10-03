import { ExternalLink, Puzzle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { serverBaseUrl } from '@/api/fetcher';
import { cn } from '@/lib/utils';
import { Badge } from './ui/badge';
import { buttonVariants } from './ui/button';
import { Item, ItemActions, ItemContent, ItemDescription, ItemMedia, ItemTitle } from './ui/item';

interface Props {
  name: string;
  // `Unknown` is a capability this server version doesn't recognise (#460)
  plugin_type: ('Processor' | 'Mapper' | 'Router' | 'Unknown')[];
  version: string;
  /** where a router plugin serves its pages, e.g. `/api/plugins/report` */
  route: string | null;
}

/** One installed plugin: name, capability badges, version, and a link to a router plugin's route. */
export default function PluginBox(props: Props) {
  const { t } = useTranslation();
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
        {props.route && (
          <a
            href={`${serverBaseUrl}${props.route}`}
            target="_blank"
            rel="noreferrer"
            title={props.route}
            className={cn(buttonVariants({ variant: 'outline', size: 'sm' }))}
          >
            {t('settings.open_plugin')}
            <ExternalLink data-icon="inline-end" />
          </a>
        )}
        <Badge variant="outline" className="font-mono tabular-nums">
          v{props.version}
        </Badge>
      </ItemActions>
    </Item>
  );
}
