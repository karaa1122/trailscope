import { Empty } from '../components/ui';

/** Screens that land in later milestones render a short note instead of a 404. */
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
