import type { Chip, ChipScalar, QueryInput, QueryMode } from './types';

/** The editable query in any of the three modes. */
export interface QueryValue {
  mode: QueryMode;
  text: string;
  chips: Chip[];
}

export const EMPTY_QUERY: QueryValue = { mode: 'sql', text: '', chips: [] };

export function toInput(v: QueryValue): QueryInput {
  return v.mode === 'filters' ? { mode: 'filters', chips: v.chips } : { mode: v.mode, text: v.text };
}

export function isEmpty(v: QueryValue): boolean {
  return v.mode === 'filters' ? v.chips.length === 0 : v.text.trim() === '';
}

/** Query state as stored in the URL. */
export interface QuerySearch {
  q?: string;
  mode?: QueryMode;
  chips?: string;
}

export function fromSearch(s: QuerySearch, fallbackMode: QueryMode = 'sql'): QueryValue {
  const mode = s.mode ?? fallbackMode;
  let chips: Chip[] = [];
  if (s.chips) {
    try {
      const parsed: unknown = JSON.parse(s.chips);
      if (Array.isArray(parsed)) chips = parsed as Chip[];
    } catch {
      chips = [];
    }
  }
  return { mode, text: s.q ?? '', chips };
}

export function toSearch(v: QueryValue): QuerySearch {
  if (v.mode === 'filters') return { mode: 'filters', chips: v.chips.length ? JSON.stringify(v.chips) : undefined, q: undefined };
  return { mode: v.mode, q: v.text || undefined, chips: undefined };
}

export function parseQuerySearch(raw: Record<string, unknown>): QuerySearch {
  const mode = raw.mode === 'sql' || raw.mode === 'search' || raw.mode === 'filters' ? raw.mode : undefined;
  return {
    q: typeof raw.q === 'string' ? raw.q : undefined,
    mode,
    chips: typeof raw.chips === 'string' ? raw.chips : undefined,
  };
}

function sqlLiteral(v: ChipScalar): string {
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return `'${v.replace(/\\/g, '\\\\').replace(/'/g, "''")}'`;
}

function searchValue(v: ChipScalar): string {
  const s = String(v);
  return /^[^\s",*\\]+$/.test(s) && !/^[<>!=]/.test(s) ? s : `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

function sqlField(c: Pick<Chip, 'field' | 'key'>): string {
  if (!c.key) return c.field;
  const map = c.field === 'attributes' ? 'attr' : c.field;
  return `${map}['${c.key.replace(/'/g, "''")}']`;
}

function searchField(c: Pick<Chip, 'field' | 'key'>): string {
  if (!c.key) return c.field;
  return `${c.field === 'attributes' ? 'attr' : c.field}.${c.key}`;
}

/** Adds a condition to the query in whatever mode it's in. */
export function addCondition(v: QueryValue, chip: Chip): QueryValue {
  if (v.mode === 'filters') {
    return { ...v, chips: [...v.chips.filter((c) => !(c.field === chip.field && c.key === chip.key && c.op === chip.op)), chip] };
  }
  const scalar = (Array.isArray(chip.value) ? chip.value[0] : chip.value) ?? '';
  if (v.mode === 'search') {
    const f = searchField(chip);
    const ops: Partial<Record<Chip['op'], string>> = { '=': '', '!=': '!', '>': '>', '>=': '>=', '<': '<', '<=': '<=' };
    let term: string;
    if (chip.op === 'in' && Array.isArray(chip.value)) term = `${f}:${chip.value.map(searchValue).join(',')}`;
    else if (chip.op in ops) term = `${f}:${ops[chip.op] ?? ''}${searchValue(scalar)}`;
    else if (chip.op === 'contains') term = `${f}:*${String(scalar)}*`;
    else term = `${f}:${searchValue(scalar)}`;
    return { ...v, text: v.text.trim() ? `${v.text.trim()} ${term}` : term };
  }
  const f = sqlField(chip);
  let cond: string;
  switch (chip.op) {
    case 'in':
    case 'not_in':
      cond = `${f} ${chip.op === 'in' ? 'IN' : 'NOT IN'} (${(Array.isArray(chip.value) ? chip.value : [scalar]).map(sqlLiteral).join(', ')})`;
      break;
    case 'contains':
    case 'not_contains':
      cond = `${f} ${chip.op === 'contains' ? 'ILIKE' : 'NOT ILIKE'} ${sqlLiteral(`%${String(scalar)}%`)}`;
      break;
    case 'like':
    case 'not_like':
      cond = `${f} ${chip.op === 'like' ? 'LIKE' : 'NOT LIKE'} ${sqlLiteral(scalar)}`;
      break;
    case 'is_empty':
      cond = `${f} IS NULL`;
      break;
    case 'is_not_empty':
      cond = `${f} IS NOT NULL`;
      break;
    default:
      cond = `${f} ${chip.op} ${sqlLiteral(scalar)}`;
  }
  const t = v.text.trim().replace(/^where\s+/i, '');
  return { ...v, text: t ? `${t} AND ${cond}` : cond };
}

export function chipLabel(c: Chip): { field: string; op: string; value: string } {
  const opText: Record<Chip['op'], string> = {
    '=': '=',
    '!=': '!=',
    '>': '>',
    '>=': '>=',
    '<': '<',
    '<=': '<=',
    in: 'IN',
    not_in: 'NOT IN',
    contains: 'contains',
    not_contains: 'not contains',
    like: 'LIKE',
    not_like: 'NOT LIKE',
    is_empty: 'is empty',
    is_not_empty: 'is set',
  };
  const value = Array.isArray(c.value) ? c.value.join(', ') : c.value === null ? '' : String(c.value);
  return { field: sqlField(c), op: opText[c.op], value };
}

const HISTORY_KEY = 'trailscope.queryHistory';

export function loadHistory(): QueryValue[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? (parsed as QueryValue[]) : [];
  } catch {
    return [];
  }
}

export function pushHistory(v: QueryValue) {
  if (isEmpty(v)) return;
  const key = JSON.stringify(toInput(v));
  const next = [v, ...loadHistory().filter((h) => JSON.stringify(toInput(h)) !== key)].slice(0, 20);
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(next));
  } catch {
    // storage disabled
  }
}
