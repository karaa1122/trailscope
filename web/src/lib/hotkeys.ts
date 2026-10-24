import { useEffect, useRef } from 'react';

type Handler = (e: KeyboardEvent) => void;

/** Keys like "/", "space", "j", "mod+k", or two-key sequences like "g l". */
export type Bindings = Record<string, Handler>;

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable || !!target.closest('.cm-editor');
}

function keyName(e: KeyboardEvent): string {
  const k = e.key === ' ' ? 'space' : e.key.length === 1 ? e.key.toLowerCase() : e.key.toLowerCase();
  return (e.metaKey || e.ctrlKey ? 'mod+' : '') + k;
}

/**
 * Global keyboard shortcuts. Ignored while typing in inputs and the editor,
 * except bindings that use `mod+`.
 */
export function useHotkeys(bindings: Bindings, enabled = true) {
  const ref = useRef(bindings);
  ref.current = bindings;
  const pending = useRef<{ key: string; at: number } | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      const name = keyName(e);
      const typing = isTyping(e.target);
      if (typing && !name.startsWith('mod+') && name !== 'escape') return;
      const map = ref.current;
      const prev = pending.current;
      if (prev && Date.now() - prev.at < 1200) {
        const seq = `${prev.key} ${name}`;
        pending.current = null;
        if (map[seq]) {
          e.preventDefault();
          map[seq](e);
          return;
        }
      }
      if (Object.keys(map).some((k) => k.startsWith(`${name} `))) {
        pending.current = { key: name, at: Date.now() };
        return;
      }
      const h = map[name];
      if (h) {
        e.preventDefault();
        h(e);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [enabled]);
}
