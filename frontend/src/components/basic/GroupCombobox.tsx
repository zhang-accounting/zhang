import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Combobox as ComboboxPrimitive } from '@base-ui/react';
import {
  Combobox,
  ComboboxCollection,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxGroup,
  ComboboxItem,
  ComboboxLabel,
  ComboboxList,
  ComboboxTrigger,
} from '@/components/ui/combobox';
import { InputGroup, InputGroupAddon, InputGroupButton, InputGroupInput } from '@/components/ui/input-group';
import { cn } from '@/lib/utils';

interface Option {
  value: string;
  label: string;
}

interface Props {
  placeholder?: string;
  value?: string;
  onChange?: (value?: string) => void;
  options: {
    group: string;
    items: Option[];
  }[];
  className?: string;
  /** Forwarded to the text input so a `<label htmlFor>` / `aria-*` names this control. */
  id?: string;
  'aria-label'?: string;
  'aria-labelledby'?: string;
  'aria-describedby'?: string;
}

/**
 * Searchable single-select over grouped string options (e.g. accounts grouped by type),
 * built on the shadcn/Base UI Combobox. Keeps the pre-Base-UI `Combobox` props.
 */
export function GroupCombobox({ placeholder, value, onChange, options, className, ...inputProps }: Props) {
  const { t } = useTranslation();
  const groups = useMemo(() => options.map((group) => ({ value: group.group, items: group.items })), [options]);
  const selected = useMemo(() => options.flatMap((group) => group.items).find((item) => item.value === value) ?? null, [options, value]);

  return (
    <Combobox<Option>
      items={groups}
      value={selected}
      onValueChange={(item) => onChange?.(item?.value)}
      itemToStringLabel={(item) => item.label}
      itemToStringValue={(item) => item.value}
      isItemEqualToValue={(item, other) => item.value === other.value}
    >
      {/* Same markup as ui/combobox's ComboboxInput, but the chevron trigger gets an accessible name. */}
      <InputGroup className={cn('w-full', className)}>
        <ComboboxPrimitive.Input render={<InputGroupInput />} placeholder={placeholder ?? t('ledger.txn.account_placeholder')} {...inputProps} />
        <InputGroupAddon align="inline-end">
          <InputGroupButton
            size="icon-xs"
            variant="ghost"
            render={<ComboboxTrigger />}
            aria-label={t('ledger.common.show_options')}
            className="data-pressed:bg-transparent"
          />
        </InputGroupAddon>
      </InputGroup>
      <ComboboxContent>
        <ComboboxEmpty>{t('ledger.common.no_match')}</ComboboxEmpty>
        <ComboboxList>
          {(group: (typeof groups)[number]) => (
            <ComboboxGroup key={group.value} items={group.items}>
              <ComboboxLabel>{group.value}</ComboboxLabel>
              <ComboboxCollection>
                {(item: Option) => (
                  <ComboboxItem key={item.value} value={item}>
                    {item.label}
                  </ComboboxItem>
                )}
              </ComboboxCollection>
            </ComboboxGroup>
          )}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}
