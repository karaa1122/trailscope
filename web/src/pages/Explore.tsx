import * as Dropdown from '@radix-ui/react-dropdown-menu';
import { keepPreviousData, useInfiniteQuery, useQuery } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useMemo, useRef, useState } from 'react';

import { Histogram, Legend } from '../components/charts';
import { QueryBar } from '../components/QueryBar';
import { TimeRange } from '../components/TimeRange';
import { Empty, Segmented, Skeleton, StatusPill, Toast, useToast } from '../components/ui';
import { ApiError, api } from '../lib/api';
import { fmtClock, fmtCompact, fmtInt, fmtMs, fmtPct, num, rangeMs, str } from '../lib/format';
import { addCondition, fromSearch, toInput, toSearch, type QuerySearch, type QueryValue } from '../lib/query';
import { useProject, useSession } from '../lib/session';
import type { ColumnInfo, Row } from '../lib/types';

export interface ExploreSearch extends QuerySearch {
  range?: string;
  from?: number;
  to?: number;
  view?: 'rows' | 'group';
  group?: string;
  cols?: string;
}

const DEFAULT_COLS = ['timestamp', 'method', 'status', 'route', 'duration_ms', 'user_id', 'client', 'service'];
const FACETS = ['service', 'status', 'method', 'route', 'environment', 'client', 'release', 'error_type'];

/** Keeps "now" fixed between refetches so relative ranges don't drift per request. */
function useRange(search: ExploreSearch): { from: number; to: number; absolute: boolean } {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => setNow(Date.now()), [search.q, search.mode, search.chips, search.range]);
  if (search.from && search.to) return { from: search.from, to: search.to, absolute: true };
  return { from: now - rangeMs(search.range, '1h'), to: now, absolute: false };
}

export function Explore({ search }: { search: ExploreSearch }) {
  const project = useProject();
  const { environment } = useSession();
  const navigate = useNavigate();
  const value = useMemo(() => fromSearch(search, 'sql'), [search]);
  const range = useRange(search);
  const view = search.view ?? 'rows';
  const cols = search.cols ? search.cols.split(',') : DEFAULT_COLS;
  const [toast, setToast] = useToast();
  const schema = useQuery({ queryKey: ['schema'], queryFn: api.schema, staleTime: Infinity });

  const set = (patch: Partial<ExploreSearch>) =>
    void navigate({ to: '/explore', search: { ...search, ...patch } });

  const scope = {
    project: project.id,
    query: toInput(value),
    from: range.from,
    to: range.to,
    environment,
  };
  const key = [project.id, environment, JSON.stringify(scope.query), range.from, range.to];

  const hist = useQuery({
    queryKey: ['histogram', ...key],
    queryFn: () => api.histogram({ ...scope, buckets: 80 }),
    placeholderData: keepPreviousData,
  });
  const releases = useQuery({
    queryKey: ['releases', project.id, environment, range.from, range.to],
    queryFn: () => api.releases(project.id, range.from, range.to, environment),
  });
  const facets = useQuery({
    queryKey: ['facets', ...key],
    queryFn: () => api.facets({ ...scope, fields: FACETS.map((column) => ({ column })), limit: 8 }),
    placeholderData: keepPreviousData,
  });
  const rows = useInfiniteQuery({
    queryKey: ['rows', ...key, cols.join(',')],
    queryFn: ({ pageParam }) => api.rows({ ...scope, columns: Array.from(new Set(['request_id', ...cols])), limit: 200, cursor: pageParam }),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    enabled: view === 'rows',
  });
  const groupBy = search.group ?? 'route';
  const groups = useQuery({
    queryKey: ['group', ...key, groupBy],
    queryFn: () => api.group({ ...scope, by: groupBy.includes('.') ? { column: groupBy.split('.')[0]!, key: groupBy.split('.').slice(1).join('.') } : { column: groupBy }, limit: 100 }),
    enabled: view === 'group',
  });

  const queryError = [hist.error, rows.error, groups.error].find((e) => e instanceof ApiError) as ApiError | undefined;
  const allRows = useMemo(() => rows.data?.pages.flatMap((p) => p.rows) ?? [], [rows.data]);
  const columns: ColumnInfo[] = schema.data?.tables.requests ?? [];

  const addFacet = (column: string, v: string) => {
    const n = Number(v);
    const next = addCondition(value, { field: column, op: '=', value: column === 'status' && !Number.isNaN(n) ? n : v });
    set({ ...toSearch(next) });
  };

  const saveView = async (v: QueryValue) => {
    const name = window.prompt('Name this view');
    if (!name) return;
    try {
      await api.createView({
        project: project.id,
        name,
        table: 'requests',
        query: toInput(v),
        timeRange: range.absolute ? { from: range.from, to: range.to } : { relative: search.range ?? '1h' },
        columns: cols,
        shared: false,
      });
      setToast(`Saved “${name}”`);
    } catch (e) {
      setToast(e instanceof Error ? e.message : 'Could not save the view');
    }
  };

  return (
    <main className="main">
      <QueryBar
        value={value}
        onRun={(v) => set({ ...toSearch(v) })}
        range={range}
        timing={rows.data?.pages[0] ? { elapsedMs: rows.data.pages[0].elapsedMs } : null}
        onSaveView={saveView}
        title="Explore"
        actions={
          <>
            <Segmented
              label="View"
              value={view}
              onChange={(v) => set({ view: v })}
              options={[
                { id: 'rows', label: 'Requests' },
                { id: 'group', label: 'Group by' },
              ]}
            />
            <TimeRange
              value={search.range ?? '1h'}
              absolute={range.absolute ? range : null}
              onClearAbsolute={() => set({ from: undefined, to: undefined })}
              onChange={(id) => set({ range: id, from: undefined, to: undefined })}
            />
          </>
        }
      />

      {queryError && (
        <div className="panel panel-pad err-text" role="alert">
          {queryError.message}
        </div>
      )}

      <section aria-label="Histogram" className="panel panel-pad" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
        <div className="panel-head">
          <h2 className="panel-title">{hist.data ? `${fmtInt(hist.data.total)} requests` : 'Requests'}</h2>
          <span className="panel-sub">drag to zoom · counts include sampling</span>
          <span style={{ marginLeft: 'auto' }}>{hist.data && <Legend series={hist.data.series} />}</span>
        </div>
        {hist.data ? (
          <Histogram
            series={hist.data.series}
            points={hist.data.points}
            bucketMs={hist.data.bucketMs}
            from={range.from}
            to={range.to}
            releases={releases.data ?? []}
            onZoom={(from, to) => set({ from: Math.round(from), to: Math.round(to) })}
          />
        ) : (
          <Skeleton h={150} />
        )}
      </section>

      <div className="row wrap" style={{ gap: 16, alignItems: 'flex-start' }}>
        <aside aria-label="Facets" className="panel" style={{ flex: '1 1 240px', maxWidth: '100%', minWidth: 0 }}>
          {FACETS.map((f) => {
            const facet = facets.data?.facets.find((x) => x.field === f);
            const total = facet?.values.reduce((a, b) => a + b.count, 0) ?? 0;
            return (
              <div key={f} style={{ padding: '10px 14px', borderBottom: '1px solid var(--line-soft)' }}>
                <div className="section-title" style={{ padding: 0 }}>
                  {f.replace('_', ' ')}
                </div>
                {!facet && <Skeleton h={12} />}
                {facet?.values.length === 0 && <span className="dim" style={{ fontSize: 12 }}>—</span>}
                {facet?.values.map((v) => (
                  <button
                    key={v.value}
                    type="button"
                    onClick={() => addFacet(f, v.value)}
                    className="menu-item"
                    style={{ minHeight: 28, padding: '0 4px', position: 'relative', borderRadius: 6 }}
                    title={`Add ${f} = ${v.value}`}
                  >
                    <span
                      aria-hidden="true"
                      style={{ position: 'absolute', left: 0, top: 4, bottom: 4, width: `${total ? (v.count / total) * 100 : 0}%`, background: 'var(--raised)', borderRadius: 4 }}
                    />
                    <span className="mono ellipsis" style={{ position: 'relative', fontSize: 12 }}>
                      {f === 'status' ? <StatusPill status={Number(v.value)} /> : v.value || '(empty)'}
                    </span>
                    <span className="nav-hint" style={{ position: 'relative' }}>
                      {fmtCompact(v.count)}
                    </span>
                  </button>
                ))}
              </div>
            );
          })}
        </aside>

        <section aria-label="Results" className="panel" style={{ flex: '999 1 560px', minWidth: 0, overflow: 'hidden' }}>
          {view === 'rows' ? (
            <>
              <div className="panel-bar">
                <h2 className="panel-title">Requests</h2>
                <span className="panel-sub">
                  {fmtInt(allRows.length)} loaded{rows.hasNextPage ? ' · scroll for more' : ''}
                </span>
                <span style={{ marginLeft: 'auto' }}>
                  <ColumnChooser columns={columns} selected={cols} onChange={(c) => set({ cols: c.join(',') })} />
                </span>
              </div>
              <ResultsTable
                rows={allRows}
                cols={cols}
                columns={columns}
                loading={rows.isLoading}
                hasMore={!!rows.hasNextPage}
                loadMore={() => void rows.fetchNextPage()}
                onOpen={(r) =>
                  void navigate({
                    to: '/requests/$requestId',
                    params: { requestId: str(r.request_id) },
                    search: { ...toSearch(value), from: range.from, to: range.to },
                  })
                }
              />
            </>
          ) : (
            <GroupView
              columns={columns}
              groupBy={groupBy}
              onGroupBy={(g) => set({ group: g })}
              loading={groups.isLoading}
              data={groups.data?.groups ?? []}
              onPick={(v) => {
                const [column, key] = groupBy.includes('.') ? [groupBy.split('.')[0]!, groupBy.split('.').slice(1).join('.')] : [groupBy, undefined];
                const n = Number(v);
                set({ ...toSearch(addCondition(value, { field: column, key, op: '=', value: column === 'status' && !Number.isNaN(n) ? n : v })), view: 'rows' });
              }}
            />
          )}
        </section>
      </div>
      <Toast msg={toast} />
    </main>
  );
}

function ColumnChooser({ columns, selected, onChange }: { columns: ColumnInfo[]; selected: string[]; onChange: (c: string[]) => void }) {
  const { me } = useSession();
  const can = me?.user.role === 'owner' || !!me?.user.can_view_bodies;
  return (
    <Dropdown.Root>
      <Dropdown.Trigger asChild>
        <button type="button" className="btn btn-sm">
          Columns
        </button>
      </Dropdown.Trigger>
      <Dropdown.Portal>
        <Dropdown.Content className="popover" sideOffset={6} align="end">
          {columns
            .filter((c) => c.kind !== 'map' && (!c.sensitive || can))
            .map((c) => (
              <Dropdown.CheckboxItem
                key={c.name}
                className="menu-item"
                checked={selected.includes(c.name)}
                onSelect={(e) => e.preventDefault()}
                onCheckedChange={(on) => onChange(on ? [...selected, c.name] : selected.filter((s) => s !== c.name))}
              >
                <span style={{ width: 14 }}>{selected.includes(c.name) ? '✓' : ''}</span>
                <span className="mono">{c.name}</span>
                <span className="nav-hint">{c.ch_type}</span>
              </Dropdown.CheckboxItem>
            ))}
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}

function width(col: string): string {
  switch (col) {
    case 'timestamp':
      return '118px';
    case 'method':
      return '66px';
    case 'status':
      return '62px';
    case 'route':
    case 'path':
    case 'error_message':
    case 'user_agent':
      return 'minmax(200px, 1fr)';
    case 'duration_ms':
      return '96px';
    default:
      return '120px';
  }
}

function Cell({ col, value }: { col: string; value: unknown }) {
  if (col === 'timestamp') return <span className="cell-mono muted">{fmtClock(num(value))}</span>;
  if (col === 'status') return <StatusPill status={num(value)} />;
  if (col === 'duration_ms') return <span className="cell-mono" style={{ textAlign: 'right' }}>{fmtMs(num(value))}</span>;
  if (col === 'method') return <span className="cell-mono" style={{ fontWeight: 600, color: 'var(--text-2)' }}>{str(value)}</span>;
  const s = typeof value === 'object' && value !== null ? JSON.stringify(value) : str(value);
  return (
    <span className={`cell-mono ellipsis${col === 'route' ? '' : ' muted'}`} style={col === 'route' ? { fontSize: 13 } : undefined} title={s}>
      {s || '—'}
    </span>
  );
}

function ResultsTable({
  rows,
  cols,
  loading,
  hasMore,
  loadMore,
  onOpen,
}: {
  rows: Row[];
  cols: string[];
  columns: ColumnInfo[];
  loading: boolean;
  hasMore: boolean;
  loadMore: () => void;
  onOpen: (r: Row) => void;
}) {
  const parent = useRef<HTMLDivElement>(null);
  const v = useVirtualizer({ count: rows.length, getScrollElement: () => parent.current, estimateSize: () => 40, overscan: 15 });
  const items = v.getVirtualItems();
  const last = items[items.length - 1];
  useEffect(() => {
    if (last && last.index >= rows.length - 20 && hasMore) loadMore();
  }, [last, rows.length, hasMore, loadMore]);
  const template = cols.map(width).join(' ');
  if (loading) {
    return (
      <div style={{ padding: 18, display: 'flex', flexDirection: 'column', gap: 10 }}>
        {[0, 1, 2, 3, 4, 5].map((i) => (
          <Skeleton key={i} h={20} />
        ))}
      </div>
    );
  }
  if (rows.length === 0) {
    return <Empty title="No matching requests">Widen the time range or remove a condition.</Empty>;
  }
  return (
    <div className="scroll-x">
      <div style={{ minWidth: Math.max(760, cols.length * 120) }}>
        <div className="table-head" style={{ gridTemplateColumns: template }}>
          {cols.map((c) => (
            <span key={c}>{c.replace('_ms', '').replace('_', ' ')}</span>
          ))}
        </div>
        <div ref={parent} style={{ height: 620, overflowY: 'auto' }}>
          <div style={{ height: v.getTotalSize(), position: 'relative' }}>
            {items.map((item) => {
              const r = rows[item.index]!;
              return (
                <div
                  key={item.key}
                  role="link"
                  tabIndex={0}
                  className={`table-row${num(r.status) >= 500 ? ' err' : ''}`}
                  onClick={() => onOpen(r)}
                  onKeyDown={(e) => e.key === 'Enter' && onOpen(r)}
                  style={{ gridTemplateColumns: template, position: 'absolute', top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` }}
                >
                  {cols.map((c) => (
                    <Cell key={c} col={c} value={r[c]} />
                  ))}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

function GroupView({
  columns,
  groupBy,
  onGroupBy,
  data,
  loading,
  onPick,
}: {
  columns: ColumnInfo[];
  groupBy: string;
  onGroupBy: (g: string) => void;
  data: { value: string; count: number; errorRate: number; p50: number; p95: number; p99: number }[];
  loading: boolean;
  onPick: (v: string) => void;
}) {
  const max = Math.max(1, ...data.map((d) => d.count));
  const cols = '180px minmax(120px, 1fr) 90px 90px 90px 90px';
  return (
    <>
      <div className="panel-bar">
        <h2 className="panel-title">Group by</h2>
        <select className="input mono" style={{ width: 220, minHeight: 36 }} value={groupBy} onChange={(e) => onGroupBy(e.target.value)} aria-label="Group by column">
          {columns
            .filter((c) => c.groupable && !c.sensitive)
            .map((c) => (
              <option key={c.name} value={c.name}>
                {c.name}
              </option>
            ))}
        </select>
        <span className="panel-sub">count · 5xx rate · latency percentiles</span>
      </div>
      {loading ? (
        <div style={{ padding: 18 }}>
          <Skeleton h={160} />
        </div>
      ) : data.length === 0 ? (
        <Empty title="Nothing to group">No requests match in this range.</Empty>
      ) : (
        <div className="scroll-x">
          <div style={{ minWidth: 760 }}>
            <div className="table-head" style={{ gridTemplateColumns: cols }}>
              <span>{groupBy}</span>
              <span>Count</span>
              <span>5xx</span>
              <span>p50</span>
              <span>p95</span>
              <span>p99</span>
            </div>
            {data.map((g) => (
              <div key={g.value} role="button" tabIndex={0} className="table-row" style={{ gridTemplateColumns: cols }} onClick={() => onPick(g.value)} onKeyDown={(e) => e.key === 'Enter' && onPick(g.value)}>
                <span className="mono ellipsis" style={{ fontSize: 13 }} title={g.value}>
                  {g.value || '(empty)'}
                </span>
                <span className="row" style={{ gap: 10 }}>
                  <span className="bar">
                    <span style={{ width: `${(g.count / max) * 100}%`, background: 'var(--ok)' }} />
                  </span>
                  <span className="cell-mono" style={{ width: 60, textAlign: 'right' }}>
                    {fmtCompact(g.count)}
                  </span>
                </span>
                <span className="cell-mono" style={{ color: g.errorRate >= 0.05 ? 'var(--err-text)' : 'var(--text-2)' }}>
                  {fmtPct(g.errorRate)}
                </span>
                <span className="cell-mono">{fmtMs(g.p50)}</span>
                <span className="cell-mono">{fmtMs(g.p95)}</span>
                <span className="cell-mono">{fmtMs(g.p99)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </>
  );
}
