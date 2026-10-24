import * as Dialog from '@radix-ui/react-dialog';
import * as Dropdown from '@radix-ui/react-dropdown-menu';
import * as Popover from '@radix-ui/react-popover';
import { useQuery } from '@tanstack/react-query';
import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { api } from '../lib/api';
import { fmtCompact, fmtInt } from '../lib/format';
import { addCondition, chipLabel, isEmpty, loadHistory, pushHistory, toInput, type QueryValue } from '../lib/query';
import { useSession } from '../lib/session';
import type { Chip, ChipOp, QueryErrorPos, QueryMode, TableName, ValidateResponse } from '../lib/types';
import { IconChevron, IconSearch } from './icons';
import { Segmented, Skeleton } from './ui';

const SqlEditor = lazy(() => import('./SqlEditor'));

const MODES: { id: QueryMode; label: string }[] = [
  { id: 'search', label: 'Search' },
  { id: 'filters', label: 'Filters' },
  { id: 'sql', label: 'SQL WHERE' },
];

export interface QueryBarProps {
  table?: TableName;
  value: QueryValue;
  /** Applies the query (Run, ⌘↵, Enter in search, chip edits). */
  onRun: (v: QueryValue) => void;
  live?: boolean;
  range?: { from: number; to: number };
  /** Timing of the last run, for the status line. */
  timing?: { elapsedMs: number; readRows?: number } | null;
  onSaveView?: (v: QueryValue) => void;
  title: string;
  /** Rendered at the right of the header row (time range, Live button). */
  actions?: ReactNode;
}

export function QueryBar({ table = 'requests', value, onRun, live = false, range, timing, onSaveView, title, actions }: QueryBarProps) {
  const { project, me } = useSession();
  const projectId = project?.id ?? '';
  const [draft, setDraft] = useState<QueryValue>(value);
  const [validation, setValidation] = useState<ValidateResponse | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const schema = useQuery({ queryKey: ['schema'], queryFn: api.schema, staleTime: Infinity });

  // Follow external changes (URL navigation, quick filters).
  useEffect(() => setDraft(value), [value]);

  // Debounced validation, 250 ms after the last edit.
  const seq = useRef(0);
  useEffect(() => {
    const id = ++seq.current;
    const t = window.setTimeout(() => {
      api
        .validate({
          table,
          query: toInput(draft),
          live,
          project: live || !projectId ? undefined : projectId,
          from: range?.from,
          to: range?.to,
        })
        .then((r) => {
          if (seq.current === id) setValidation(r);
        })
        .catch(() => undefined);
    }, 250);
    return () => window.clearTimeout(t);
  }, [draft, table, live, projectId, range?.from, range?.to]);

  const run = useCallback(
    (v: QueryValue = draft) => {
      pushHistory(v);
      onRun(v);
    },
    [draft, onRun],
  );

  const switchMode = (mode: QueryMode) => {
    if (mode === draft.mode) return;
    setNotice(null);
    if (isEmpty(draft)) {
      setDraft({ ...draft, mode });
      return;
    }
    if (!validation?.ok) {
      setNotice('Fix the query first, then switch modes.');
      return;
    }
    if (mode === 'sql') {
      run({ mode, text: validation.sql ?? '', chips: draft.chips });
    } else if (mode === 'search') {
      if (validation.search === null) setNotice('This query can only be edited as SQL.');
      else run({ mode, text: validation.search, chips: draft.chips });
    } else {
      if (validation.chips === null) setNotice('This query can only be edited as SQL.');
      else run({ mode, text: draft.text, chips: validation.chips });
    }
  };

  const errors: QueryErrorPos[] = validation && !validation.ok ? validation.errors : [];
  const dirty = JSON.stringify(toInput(draft)) !== JSON.stringify(toInput(value));

  return (
    <>
      <div className="page-header">
        <h1 className="page-title">{title}</h1>
        <Segmented label="Query mode" options={MODES} value={draft.mode} onChange={switchMode} />
        <span className="grow" />
        {actions}
      </div>
      <section aria-label="Query" className="panel query-card">
        {draft.mode === 'sql' && (
          <div className="row wrap" style={{ alignItems: 'stretch', gap: 10 }}>
            <div className="editor-box">
              <Suspense fallback={<div style={{ padding: 10 }}><Skeleton h={22} /></div>}>
                <SqlEditor
                  value={draft.text}
                  onChange={(text) => setDraft((d) => ({ ...d, text }))}
                  onRun={() => run()}
                  errors={errors}
                  schema={schema.data}
                  table={table}
                  live={live}
                  allowSensitive={!!me && (me.user.role === 'owner' || me.user.can_view_bodies)}
                  fetchValues={async (column, prefix) =>
                    projectId ? (await api.values(projectId, column, prefix, table)).values.map((v) => v.value) : []
                  }
                  fetchKeys={async (map) => (projectId ? (await api.mapKeys(projectId, map, table)).values.map((v) => v.value) : [])}
                />
              </Suspense>
            </div>
            <div style={{ flex: '0 1 160px', display: 'flex', flexDirection: 'column', gap: 8 }}>
              <button type="button" className="btn btn-primary" onClick={() => run()}>
                Run <span className="mono" style={{ fontSize: 11, opacity: 0.7 }}>⌘↵</span>
              </button>
              {onSaveView && (
                <button type="button" className="btn btn-ghost" onClick={() => onSaveView(draft)}>
                  Save as view
                </button>
              )}
            </div>
          </div>
        )}

        {draft.mode === 'search' && (
          <SearchInput
            value={draft.text}
            invalid={errors.length > 0}
            onChange={(text) => setDraft((d) => ({ ...d, text }))}
            onSubmit={() => run()}
          />
        )}

        {draft.mode === 'filters' && (
          <div className="row wrap">
            {draft.chips.map((c, i) => {
              const l = chipLabel(c);
              return (
                <span key={`${c.field}-${c.key ?? ''}-${i}`} className="filter-chip">
                  <span className="muted">{l.field}</span>
                  <span style={{ color: 'var(--accent)' }}>{l.op}</span>
                  <span>{l.value}</span>
                  <button
                    type="button"
                    className="x"
                    aria-label={`Remove filter ${l.field} ${l.op} ${l.value}`}
                    onClick={() => run({ ...draft, chips: draft.chips.filter((_, j) => j !== i) })}
                  >
                    ×
                  </button>
                </span>
              );
            })}
            <AddFilter table={table} live={live} onAdd={(chip) => run({ ...draft, chips: [...draft.chips, chip] })} />
            <span className="muted" style={{ marginLeft: 'auto', fontSize: 12 }}>
              AND · switch to SQL to edit as text
            </span>
          </div>
        )}

        <div className="status-line">
          {errors.length > 0 ? (
            <span className="err-text" role="alert">
              ✗ {errors[0]!.message}
              {draft.mode === 'sql' && <span className="muted"> (line {errors[0]!.line}, column {errors[0]!.col})</span>}
            </span>
          ) : validation?.ok ? (
            <>
              <span className="ok-text">✓ valid</span>
              <span>
                {validation.conditions === 0 ? 'no filter · everything in range' : `${validation.conditions} condition${validation.conditions === 1 ? '' : 's'}`}
                {validation.estimatedRows !== null && ` · ~${fmtCompact(validation.estimatedRows)} rows to scan`}
                {timing && ` · ${fmtInt(timing.elapsedMs)} ms`}
                {dirty && ' · not run yet'}
              </span>
            </>
          ) : (
            <span>checking…</span>
          )}
          <span>read-only · whitelisted columns · scoped to this project</span>
          {notice && <span style={{ color: 'var(--warn)' }}>{notice}</span>}
          <span style={{ marginLeft: 'auto', display: 'flex', gap: 14 }}>
            <History onPick={(v) => run(v)} />
            <SyntaxHelp />
          </span>
        </div>

        <QuickFilters
          table={table}
          query={value}
          range={range}
          live={live}
          onAdd={(chip) => run(addCondition(draft, chip))}
        />
      </section>
    </>
  );
}

function SearchInput({
  value,
  invalid,
  onChange,
  onSubmit,
}: {
  value: string;
  invalid: boolean;
  onChange: (v: string) => void;
  onSubmit: () => void;
}) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const focus = () => ref.current?.focus();
    window.addEventListener('logbox:focus-query', focus);
    return () => window.removeEventListener('logbox:focus-query', focus);
  }, []);
  return (
    <div className={`search-box${invalid ? ' invalid' : ''}`}>
      <IconSearch />
      <label htmlFor="lb-q" className="sr-only">
        Search
      </label>
      <input
        id="lb-q"
        ref={ref}
        type="text"
        value={value}
        placeholder='status:>=500 route:/api/* "free text"'
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onSubmit();
        }}
        spellCheck={false}
      />
      <span className="mono muted" style={{ fontSize: 11, border: '1px solid var(--line-strong)', borderRadius: 6, padding: '2px 7px' }}>
        ↵
      </span>
    </div>
  );
}

const OPS: { id: ChipOp; label: string; needsValue: boolean }[] = [
  { id: '=', label: '=', needsValue: true },
  { id: '!=', label: '!=', needsValue: true },
  { id: '>', label: '>', needsValue: true },
  { id: '>=', label: '>=', needsValue: true },
  { id: '<', label: '<', needsValue: true },
  { id: '<=', label: '<=', needsValue: true },
  { id: 'in', label: 'in (comma-separated)', needsValue: true },
  { id: 'contains', label: 'contains', needsValue: true },
  { id: 'like', label: 'LIKE', needsValue: true },
  { id: 'is_empty', label: 'is empty', needsValue: false },
  { id: 'is_not_empty', label: 'is set', needsValue: false },
];

function AddFilter({ table, live, onAdd }: { table: TableName; live: boolean; onAdd: (c: Chip) => void }) {
  const { me } = useSession();
  const schema = useQuery({ queryKey: ['schema'], queryFn: api.schema, staleTime: Infinity });
  const [open, setOpen] = useState(false);
  const [field, setField] = useState('status');
  const [key, setKey] = useState('');
  const [op, setOp] = useState<ChipOp>('=');
  const [value, setValue] = useState('');
  const cols = (schema.data?.tables[table] ?? []).filter(
    (c) => c.filterable && (!c.sensitive || me?.user.role === 'owner' || me?.user.can_view_bodies) && (!live || c.live),
  );
  const col = cols.find((c) => c.name === field);
  const needsValue = OPS.find((o) => o.id === op)?.needsValue ?? true;
  const submit = () => {
    const parse = (s: string) => (col?.kind === 'number' && s.trim() !== '' && !Number.isNaN(Number(s)) ? Number(s) : s.trim());
    const chipValue = !needsValue ? null : op === 'in' ? value.split(',').map(parse).filter((v) => v !== '') : parse(value);
    onAdd({ field, key: col?.kind === 'map' ? key : undefined, op, value: chipValue });
    setOpen(false);
    setValue('');
  };
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <button type="button" className="btn-dashed">
          + Add filter
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="popover" sideOffset={6} align="start" style={{ padding: 12, width: 300 }}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
            style={{ display: 'flex', flexDirection: 'column', gap: 10 }}
          >
            <label className="field">
              Field
              <select className="input" value={field} onChange={(e) => setField(e.target.value)}>
                {cols.map((c) => (
                  <option key={c.name} value={c.name}>
                    {c.name}
                  </option>
                ))}
              </select>
            </label>
            {col?.kind === 'map' && (
              <label className="field">
                Key
                <input className="input mono" value={key} onChange={(e) => setKey(e.target.value)} placeholder="tenant" />
              </label>
            )}
            <label className="field">
              Operator
              <select className="input" value={op} onChange={(e) => setOp(e.target.value as ChipOp)}>
                {OPS.map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            {needsValue && (
              <label className="field">
                Value
                <input className="input mono" value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
              </label>
            )}
            <button type="submit" className="btn btn-primary">
              Add
            </button>
          </form>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

interface QuickField {
  label: string;
  column: string;
  key?: string;
  hint?: string;
  presets?: { label: string; chip: Chip }[];
}

const QUICK: QuickField[] = [
  { label: 'Service', column: 'service' },
  {
    label: 'Status',
    column: 'status',
    presets: [
      { label: '2xx', chip: { field: 'status', op: '<', value: 300 } },
      { label: '≥ 400', chip: { field: 'status', op: '>=', value: 400 } },
      { label: '4xx', chip: { field: 'status', op: 'in', value: [400, 401, 403, 404, 409, 422, 429] } },
      { label: '5xx', chip: { field: 'status', op: '>=', value: 500 } },
    ],
  },
  { label: 'Method', column: 'method' },
  { label: 'Route', column: 'route' },
  { label: 'User', column: 'user_id' },
  { label: 'Client', column: 'client' },
  {
    label: 'Duration',
    column: 'duration_ms',
    presets: [100, 300, 800, 1000, 3000].map((n) => ({
      label: `> ${fmtInt(n)} ms`,
      chip: { field: 'duration_ms', op: '>', value: n } as Chip,
    })),
  },
  { label: 'Release', column: 'release' },
];

function QuickFilters({
  table,
  query,
  range,
  live,
  onAdd,
}: {
  table: TableName;
  query: QueryValue;
  range?: { from: number; to: number };
  live: boolean;
  onAdd: (c: Chip) => void;
}) {
  if (table !== 'requests') return null;
  return (
    <div className="row wrap" style={{ gap: 6 }}>
      <span className="section-title" style={{ margin: '0 4px 0 0', padding: 0 }}>
        Quick filters
      </span>
      {QUICK.map((f) => (
        <FacetButton key={f.label} field={f} query={query} range={range} onAdd={onAdd} />
      ))}
      {!live && <AttributeFacet query={query} range={range} onAdd={onAdd} />}
    </div>
  );
}

function FacetButton({
  field,
  query,
  range,
  onAdd,
}: {
  field: QuickField;
  query: QueryValue;
  range?: { from: number; to: number };
  onAdd: (c: Chip) => void;
}) {
  const { project, environment } = useSession();
  const [open, setOpen] = useState(false);
  const facets = useQuery({
    queryKey: ['facet', project?.id, environment, field.column, field.key, toInput(query), range?.from, range?.to],
    queryFn: () =>
      api.facets({
        project: project?.id ?? '',
        query: toInput(query),
        from: range?.from,
        to: range?.to,
        environment,
        fields: [{ column: field.column, key: field.key }],
        limit: 12,
      }),
    enabled: open && !!project && !field.presets,
    retry: false,
  });
  const values = facets.data?.facets[0]?.values ?? [];
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <button type="button" className="facet-btn">
          {field.label}
          <IconChevron size={12} />
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="popover" sideOffset={6} align="start">
          {field.presets?.map((p) => (
            <button
              key={p.label}
              type="button"
              className="menu-item mono"
              onClick={() => {
                onAdd(p.chip);
                setOpen(false);
              }}
            >
              {p.label}
            </button>
          ))}
          {!field.presets && facets.isLoading && (
            <div style={{ padding: 12 }}>
              <Skeleton />
            </div>
          )}
          {!field.presets && !facets.isLoading && values.length === 0 && <div className="empty">No values in range</div>}
          {values.map((v) => (
            <button
              key={v.value}
              type="button"
              className="menu-item"
              onClick={() => {
                const n = Number(v.value);
                onAdd({
                  field: field.column,
                  key: field.key,
                  op: '=',
                  value: field.column === 'status' && !Number.isNaN(n) ? n : v.value,
                });
                setOpen(false);
              }}
            >
              <span className="mono ellipsis">{v.value === '' ? '(empty)' : v.value}</span>
              <span className="nav-hint">{fmtCompact(v.count)}</span>
            </button>
          ))}
          {!field.presets && <div className="menu-foot">counts under the current filter</div>}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

function AttributeFacet({
  query,
  range,
  onAdd,
}: {
  query: QueryValue;
  range?: { from: number; to: number };
  onAdd: (c: Chip) => void;
}) {
  const { project } = useSession();
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState<string | null>(null);
  const keys = useQuery({
    queryKey: ['attr-keys', project?.id],
    queryFn: () => api.mapKeys(project?.id ?? '', 'attributes'),
    enabled: open && !!project,
  });
  return (
    <>
      <Popover.Root
        open={open}
        onOpenChange={(o) => {
          setOpen(o);
          if (!o) setKey(null);
        }}
      >
        <Popover.Trigger asChild>
          <button type="button" className="btn-dashed">
            + attribute
          </button>
        </Popover.Trigger>
        <Popover.Portal>
          <Popover.Content className="popover" sideOffset={6} align="start">
            {key === null ? (
              <>
                {keys.data?.values.map((k) => (
                  <button key={k.value} type="button" className="menu-item" onClick={() => setKey(k.value)}>
                    <span className="mono">attr['{k.value}']</span>
                    <span className="nav-hint">{fmtCompact(k.count)}</span>
                  </button>
                ))}
                {keys.data?.values.length === 0 && <div className="empty">No attributes in the last hour</div>}
              </>
            ) : (
              <AttrValues attrKey={key} query={query} range={range} onPick={(v) => {
                onAdd({ field: 'attributes', key, op: '=', value: v });
                setOpen(false);
                setKey(null);
              }} />
            )}
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>
    </>
  );
}

function AttrValues({
  attrKey,
  query,
  range,
  onPick,
}: {
  attrKey: string;
  query: QueryValue;
  range?: { from: number; to: number };
  onPick: (v: string) => void;
}) {
  const { project, environment } = useSession();
  const facets = useQuery({
    queryKey: ['facet', project?.id, environment, 'attributes', attrKey, toInput(query), range?.from, range?.to],
    queryFn: () =>
      api.facets({
        project: project?.id ?? '',
        query: toInput(query),
        from: range?.from,
        to: range?.to,
        environment,
        fields: [{ column: 'attributes', key: attrKey }],
      }),
  });
  return (
    <>
      {facets.data?.facets[0]?.values.map((v) => (
        <button key={v.value} type="button" className="menu-item" onClick={() => onPick(v.value)}>
          <span className="mono">{v.value === '' ? '(empty)' : v.value}</span>
          <span className="nav-hint">{fmtCompact(v.count)}</span>
        </button>
      ))}
    </>
  );
}

function History({ onPick }: { onPick: (v: QueryValue) => void }) {
  const [items, setItems] = useState<QueryValue[]>([]);
  return (
    <Dropdown.Root onOpenChange={(o) => o && setItems(loadHistory())}>
      <Dropdown.Trigger asChild>
        <button type="button" className="btn-sm" style={{ border: 0, color: 'var(--accent)', padding: 0, minHeight: 0, cursor: 'pointer', background: 'none' }}>
          History
        </button>
      </Dropdown.Trigger>
      <Dropdown.Portal>
        <Dropdown.Content className="popover" sideOffset={6} align="end" style={{ maxWidth: 520 }}>
          {items.length === 0 && <div className="empty">Queries you run show up here</div>}
          {items.map((h, i) => (
            <Dropdown.Item key={i} className="menu-item" onSelect={() => onPick(h)}>
              <span className="nav-hint" style={{ marginLeft: 0, width: 50 }}>
                {h.mode}
              </span>
              <span className="mono ellipsis">{h.mode === 'filters' ? h.chips.map((c) => Object.values(chipLabel(c)).join(' ')).join(' AND ') : h.text}</span>
            </Dropdown.Item>
          ))}
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}

function SyntaxHelp() {
  const examples = useMemo(
    () => [
      ["status >= 500 AND route LIKE '/api/%'", 'server errors on a route prefix'],
      ["attr['tenant'] IN ('a', 'b')", 'attributes and IN lists'],
      ["correlation['account_id'] = 'acc_1'", 'everything for one journey key'],
      ['timestamp > now() - INTERVAL 15 MINUTE', 'relative time'],
      ["match(path, '^/v[0-9]+/') AND NOT method = 'GET'", 'regex and negation'],
      ['user_id IS NULL', 'empty values'],
      ['status:5xx route:/api/* duration:>800ms "timeout"', 'search syntax'],
    ],
    [],
  );
  return (
    <Dialog.Root>
      <Dialog.Trigger asChild>
        <button type="button" style={{ border: 0, background: 'none', padding: 0, color: 'var(--accent)', cursor: 'pointer', fontSize: 12 }}>
          Syntax help
        </button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog" style={{ padding: 22 }} aria-describedby={undefined}>
          <Dialog.Title style={{ margin: '0 0 6px', fontSize: 16 }}>Query syntax</Dialog.Title>
          <p className="muted" style={{ fontSize: 13, marginTop: 0 }}>
            Write a SQL <span className="mono">WHERE</span> expression. Allowed: comparisons, AND/OR/NOT, IN, BETWEEN, LIKE/ILIKE, IS
            [NOT] NULL, <span className="mono">attr['key']</span>, <span className="mono">now() - INTERVAL n MINUTE</span> and the
            functions lower, upper, length, startsWith, endsWith, position and match. Everything else is rejected.
          </p>
          {examples.map(([q, d]) => (
            <div key={q} className="kv" style={{ padding: '8px 0' }}>
              <span className="mono" style={{ color: 'var(--text)', fontSize: 12.5 }}>
                {q}
              </span>
              <span className="muted" style={{ fontFamily: 'var(--font)' }}>
                {d}
              </span>
            </div>
          ))}
          <div className="row" style={{ justifyContent: 'flex-end', marginTop: 14 }}>
            <Dialog.Close asChild>
              <button type="button" className="btn">
                Close
              </button>
            </Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
