import * as Dropdown from '@radix-ui/react-dropdown-menu';

import { RANGES, fmtDateTime, rangeLabel } from '../lib/format';
import { IconClock } from './icons';

/** Relative range picker; an absolute range (from a zoom) shows its bounds. */
export function TimeRange({
  value,
  onChange,
  absolute,
  onClearAbsolute,
  options = RANGES.map((r) => r.id),
}: {
  value: string;
  onChange: (id: string) => void;
  absolute?: { from: number; to: number } | null;
  onClearAbsolute?: () => void;
  options?: string[];
}) {
  return (
    <Dropdown.Root>
      <Dropdown.Trigger asChild>
        <button type="button" className="btn">
          <IconClock />
          {absolute ? `${fmtDateTime(absolute.from)} → ${fmtDateTime(absolute.to)}` : rangeLabel(value)}
        </button>
      </Dropdown.Trigger>
      <Dropdown.Portal>
        <Dropdown.Content className="popover" sideOffset={6} align="end">
          {absolute && onClearAbsolute && (
            <Dropdown.Item className="menu-item" onSelect={onClearAbsolute}>
              Back to {rangeLabel(value)}
            </Dropdown.Item>
          )}
          {RANGES.filter((r) => options.includes(r.id)).map((r) => (
            <Dropdown.Item key={r.id} className="menu-item" onSelect={() => onChange(r.id)}>
              {r.label}
              {!absolute && r.id === value && <span className="nav-hint">✓</span>}
            </Dropdown.Item>
          ))}
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}
