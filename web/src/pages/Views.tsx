import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';

import { Empty, Skeleton, Toast, useToast } from '../components/ui';
import { api } from '../lib/api';
import { fmtAgo } from '../lib/format';
import { chipLabel, toSearch, type QueryValue } from '../lib/query';
import { useProject, useSession } from '../lib/session';
import type { SavedView } from '../lib/types';

function asValue(v: SavedView): QueryValue {
  return v.query.mode === 'filters' ? { mode: 'filters', text: '', chips: v.query.chips } : { mode: v.query.mode, text: v.query.text, chips: [] };
}

function describe(v: SavedView): string {
  if (v.query.mode === 'filters') {
    return v.query.chips.map((c) => Object.values(chipLabel(c)).join(' ')).join(' · ') || 'all requests';
  }
  return v.query.text || 'all requests';
}

export function Views() {
  const project = useProject();
  const { me } = useSession();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [toast, setToast] = useToast();
  const views = useQuery({ queryKey: ['views', project.id], queryFn: () => api.views(project.id) });
  const refresh = () => void qc.invalidateQueries({ queryKey: ['views', project.id] });
  const onError = (e: unknown) => setToast(e instanceof Error ? e.message : 'Something went wrong');

  const update = useMutation({
    mutationFn: (v: SavedView) =>
      api.updateView(v.id, { project: v.project, name: v.name, table: v.table, query: v.query, timeRange: v.timeRange, columns: v.columns, shared: v.shared }),
    onSuccess: refresh,
    onError,
  });
  const remove = useMutation({ mutationFn: (id: string) => api.deleteView(id), onSuccess: refresh, onError });

  const open = (v: SavedView) => {
    const tr = v.timeRange;
    void navigate({
      to: '/explore',
      search: {
        ...toSearch(asValue(v)),
        range: tr.relative,
        from: tr.relative ? undefined : tr.from,
        to: tr.relative ? undefined : tr.to,
        cols: v.columns.length ? v.columns.join(',') : undefined,
      },
    });
  };
  const canEdit = (v: SavedView) => v.ownerId === me?.user.id || me?.user.role === 'admin' || me?.user.role === 'owner';
  const cols = 'minmax(0, 1fr) minmax(0, 2fr) 90px 90px 220px';

  return (
    <main className="main" style={{ gap: 16 }}>
      <header className="page-header">
        <h1 className="page-title">Saved views</h1>
        <span className="panel-sub">save a query from Explore with “Save view”; shared views are visible to everyone in the project</span>
      </header>
      <section aria-label="Saved views" className="panel" style={{ overflow: 'hidden' }}>
        <div className="scroll-x">
          <div style={{ minWidth: 820 }}>
            <div className="table-head" style={{ gridTemplateColumns: cols }}>
              <span>Name</span>
              <span>Query</span>
              <span>Range</span>
              <span>Updated</span>
              <span />
            </div>
            {views.isLoading && <Skeleton h={160} />}
            {views.data?.length === 0 && <Empty title="No saved views yet">Run a query in Explore, then choose “Save view”.</Empty>}
            {views.data?.map((v) => (
              <div key={v.id} className="table-row" style={{ gridTemplateColumns: cols, cursor: 'default', padding: '8px 18px' }}>
                <button type="button" className="btn-ghost" onClick={() => open(v)} style={{ textAlign: 'left', border: 0, background: 'none', color: 'var(--text)', cursor: 'pointer', padding: 0, font: 'inherit', fontWeight: 500 }}>
                  {v.name}
                  {v.shared && <span className="badge steady" style={{ marginLeft: 8 }}>shared</span>}
                </button>
                <span className="cell-mono ellipsis muted" title={describe(v)}>
                  {v.query.mode !== 'sql' && <span className="dim">{v.query.mode} · </span>}
                  {describe(v)}
                </span>
                <span className="cell-mono muted">{v.timeRange.relative ?? 'fixed'}</span>
                <span className="muted" style={{ fontSize: 12 }}>
                  {fmtAgo(Date.parse(v.updatedAt))}
                </span>
                <span className="row" style={{ justifyContent: 'flex-end', gap: 6 }}>
                  <button type="button" className="btn btn-sm btn-primary" onClick={() => open(v)}>
                    Open
                  </button>
                  {canEdit(v) && (
                    <>
                      <button
                        type="button"
                        className="btn btn-sm"
                        onClick={() => {
                          const name = window.prompt('Rename view', v.name)?.trim();
                          if (name && name !== v.name) update.mutate({ ...v, name });
                        }}
                      >
                        Rename
                      </button>
                      <button type="button" className="btn btn-sm" onClick={() => update.mutate({ ...v, shared: !v.shared })}>
                        {v.shared ? 'Unshare' : 'Share'}
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm"
                        aria-label={`Delete ${v.name}`}
                        onClick={() => window.confirm(`Delete “${v.name}”?`) && remove.mutate(v.id)}
                      >
                        Delete
                      </button>
                    </>
                  )}
                </span>
              </div>
            ))}
          </div>
        </div>
      </section>
      <Toast msg={toast} />
    </main>
  );
}
