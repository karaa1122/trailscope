import { Empty } from '../components/ui';

/** A page with only a short note, e.g. when there are no projects yet. */
export function Placeholder({ title, note }: { title: string; note: string }) {
  return (
    <main className="main">
      <div className="page-header">
        <h1 className="page-title">{title}</h1>
      </div>
      <div className="panel">
        <Empty title={title}>{note}</Empty>
      </div>
    </main>
  );
}
