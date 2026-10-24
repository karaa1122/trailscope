import * as Dialog from '@radix-ui/react-dialog';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useMemo, useState } from 'react';

import { useSession } from '../lib/session';

interface Command {
  id: string;
  label: string;
  hint?: string;
  run: () => void;
}

export function CommandPalette({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const navigate = useNavigate();
  const { projects, setProject, toggleTheme, signOut, theme } = useSession();
  const [text, setText] = useState('');
  const [index, setIndex] = useState(0);

  useEffect(() => {
    if (open) {
      setText('');
      setIndex(0);
    }
  }, [open]);

  const commands = useMemo<Command[]>(() => {
    const go = (to: string, search?: Record<string, string>) => () => void navigate({ to, search });
    const list: Command[] = [
      { id: 'live', label: 'Go to Live tail', hint: 'G L', run: go('/live') },
      { id: 'explore', label: 'Go to Explore', hint: 'G E', run: go('/explore') },
      { id: 'errors', label: 'Go to Errors', hint: 'G X', run: go('/errors') },
      { id: 'journeys', label: 'Go to Journeys', hint: 'G J', run: go('/journeys') },
      { id: 'routes', label: 'Go to Routes', hint: 'G R', run: go('/routes') },
      { id: 'alerts', label: 'Go to Alerts', hint: 'G A', run: go('/alerts') },
      { id: 'views', label: 'Go to Saved views', run: go('/views') },
      { id: 'settings', label: 'Go to Settings', hint: 'G S', run: go('/settings') },
      { id: 'errors-5xx', label: 'Explore server errors (5xx)', run: go('/explore', { q: 'status:5xx', mode: 'search' }) },
      { id: 'slow', label: 'Explore slow requests', run: go('/explore', { q: 'duration_ms > 1000', mode: 'sql' }) },
      { id: 'theme', label: `Switch to ${theme === 'dark' ? 'light' : 'dark'} theme`, run: toggleTheme },
      { id: 'docs', label: 'Open API reference', run: () => window.open('/api/docs', '_blank', 'noopener') },
      { id: 'signout', label: 'Sign out', run: () => void signOut().then(() => navigate({ to: '/login' })) },
    ];
    for (const p of projects) {
      list.push({ id: `project-${p.id}`, label: `Switch to project ${p.name}`, run: () => setProject(p.id) });
    }
    const t = text.trim();
    const dynamic: Command[] = [];
    if (/^[A-Za-z0-9._:-]{6,128}$/.test(t) && !t.includes(':')) {
      dynamic.push({
        id: 'req',
        label: `Open request ${t}`,
        run: () => void navigate({ to: '/requests/$requestId', params: { requestId: t } }),
      });
    }
    const kv = /^([a-zA-Z0-9_.-]+):(.+)$/.exec(t);
    if (kv) {
      dynamic.push({ id: 'journey', label: `Journey for ${kv[1]} = ${kv[2]}`, run: go('/journeys', { q: t }) });
    }
    if (t) dynamic.push({ id: 'search', label: `Search for “${t}”`, run: go('/explore', { q: t, mode: 'search' }) });
    const lower = t.toLowerCase();
    const matched = list.filter((c) => !lower || c.label.toLowerCase().includes(lower));
    return [...dynamic, ...matched];
  }, [text, navigate, projects, setProject, toggleTheme, signOut, theme]);

  const run = (c: Command | undefined) => {
    if (!c) return;
    onOpenChange(false);
    c.run();
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog" aria-describedby={undefined}>
          <Dialog.Title className="sr-only">Command palette</Dialog.Title>
          <div style={{ padding: 10, borderBottom: '1px solid var(--line)' }}>
            <input
              autoFocus
              className="input mono"
              placeholder="Type a command, request ID, or key:value"
              value={text}
              onChange={(e) => {
                setText(e.target.value);
                setIndex(0);
              }}
              onKeyDown={(e) => {
                if (e.key === 'ArrowDown') {
                  e.preventDefault();
                  setIndex((i) => Math.min(commands.length - 1, i + 1));
                } else if (e.key === 'ArrowUp') {
                  e.preventDefault();
                  setIndex((i) => Math.max(0, i - 1));
                } else if (e.key === 'Enter') {
                  e.preventDefault();
                  run(commands[index]);
                }
              }}
              aria-label="Command"
            />
          </div>
          <div role="listbox" aria-label="Commands" style={{ padding: 6, maxHeight: 360, overflowY: 'auto' }}>
            {commands.map((c, i) => (
              <button
                key={c.id}
                type="button"
                role="option"
                aria-selected={i === index}
                className="menu-item"
                onMouseEnter={() => setIndex(i)}
                onClick={() => run(c)}
                style={{ borderRadius: 8 }}
              >
                {c.label}
                {c.hint && <span className="nav-hint">{c.hint}</span>}
              </button>
            ))}
            {commands.length === 0 && <div className="empty">No matching commands</div>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
