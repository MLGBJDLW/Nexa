import { useMemo, useState, type ReactNode } from 'react';
import { Command, defaultFilter } from 'cmdk';
import { compactModelSearch } from '../../../lib/modelSearch';
import { Check, ChevronDown } from 'lucide-react';
import {
  NexaPopover,
  NexaPopoverContent,
  NexaPopoverTrigger,
} from './Popover';

export interface NexaComboboxOption {
  value: string;
  label: string;
  description?: string;
  wrapLabel?: boolean;
  keywords?: string[];
  disabled?: boolean;
  badge?: ReactNode;
}

function filterOption(value: string, search: string, keywords: string[] = []): number {
  const query = compactModelSearch(search);
  if (query && compactModelSearch(value) === query) return 1;
  return defaultFilter(value, search.normalize('NFKC'), keywords);
}

export function NexaCombobox({
  ariaLabel,
  className = '',
  dataTestId,
  emptyLabel = 'No results',
  onValueChange,
  options,
  placeholder = 'Select…',
  searchPlaceholder = 'Search…',
  value,
  disabled = false,
}: {
  ariaLabel: string;
  className?: string;
  dataTestId?: string;
  emptyLabel?: string;
  onValueChange: (value: string) => void;
  options: NexaComboboxOption[];
  placeholder?: string;
  searchPlaceholder?: string;
  value?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const selected = useMemo(() => options.find(option => option.value === value), [options, value]);

  return (
    <NexaPopover open={open} onOpenChange={setOpen}>
      <NexaPopoverTrigger asChild>
        <button
          type="button"
          disabled={disabled}
          className={`nexa-select-trigger ${className}`}
          aria-label={ariaLabel}
          aria-expanded={open}
          data-nexa-select-trigger
          data-value={value ?? ''}
          data-testid={dataTestId}
          title={selected?.wrapLabel ? selected.label : undefined}
        >
          <span className="min-w-0 flex-1 truncate text-left">{selected?.label ?? placeholder}</span>
          <ChevronDown className="nexa-select-chevron ml-2 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        </button>
      </NexaPopoverTrigger>
      <NexaPopoverContent className="nexa-combobox-content min-w-[var(--radix-popover-trigger-width)] p-1">
        <Command loop filter={filterOption}>
          <Command.Input className="nexa-combobox-input" placeholder={searchPlaceholder} />
          <Command.List className="nexa-combobox-list p-1">
            <Command.Empty className="px-2 py-6 text-center text-xs text-text-tertiary">{emptyLabel}</Command.Empty>
            {options.map(option => (
              <Command.Item
                key={option.value}
                value={option.value}
                keywords={[option.label, option.description ?? '', ...(option.keywords ?? [])]}
                disabled={option.disabled}
                onSelect={() => {
                  onValueChange(option.value);
                  // Let the pointer/keyboard activation finish before the
                  // controlled value update unmounts the popover contents.
                  window.requestAnimationFrame(() => setOpen(false));
                }}
                className="nexa-overlay-item"
              >
                <Check className={`h-3.5 w-3.5 ${option.value === value ? 'opacity-100' : 'opacity-0'}`} />
                <span className="min-w-0 flex-1">
                  <span className={`block text-xs font-medium text-text-primary ${option.wrapLabel ? "whitespace-normal break-words" : "truncate"}`}>{option.label}</span>
                  {option.description && (
                    <span className="block truncate text-[11px] text-text-tertiary">{option.description}</span>
                  )}
                </span>
                {option.badge}
              </Command.Item>
            ))}
          </Command.List>
        </Command>
      </NexaPopoverContent>
    </NexaPopover>
  );
}
