import { useEffect, useRef, useState } from 'react';

import { liveUrl } from './api';
import type { LiveRequest } from './types';

const MAX_ROWS = 10_000;

export interface LiveState {
  rows: LiveRequest[];
  connected: boolean;
  /** Events the server dropped (rate cap or lag). */
  skipped: number;
  /** Events received while paused, not yet shown. */
  pendingWhilePaused: number;
  /** Requests per second over the last few seconds. */
  rate: number;
  error: string | null;
}

/**
 * Subscribes to the live tail. Pausing is client-side: events keep arriving
 * and are buffered, then appear when resumed.
 */
export function useLiveTail(
  project: string | null,
  filter: string,
  mode: 'sql' | 'search',
  environment: string | null,
  paused: boolean,
  enabled = true,
): LiveState {
  const [state, setState] = useState<LiveState>({
    rows: [],
    connected: false,
    skipped: 0,
    pendingWhilePaused: 0,
    rate: 0,
    error: null,
  });
  const buffer = useRef<LiveRequest[]>([]);
  const held = useRef<LiveRequest[]>([]);
  const pausedRef = useRef(paused);
  const arrivals = useRef<number[]>([]);

  useEffect(() => {
    pausedRef.current = paused;
    if (!paused && held.current.length) {
      buffer.current = [...held.current.reverse(), ...buffer.current].slice(0, MAX_ROWS);
      held.current = [];
      setState((s) => ({ ...s, rows: buffer.current, pendingWhilePaused: 0 }));
    }
  }, [paused]);

  useEffect(() => {
    if (!project || !enabled) return;
    buffer.current = [];
    held.current = [];
    setState({ rows: [], connected: false, skipped: 0, pendingWhilePaused: 0, rate: 0, error: null });
    const es = new EventSource(liveUrl(project, filter, mode, environment), { withCredentials: true });
    let incoming: LiveRequest[] = [];
    let skipped = 0;
    const flush = () => {
      const now = Date.now();
      arrivals.current = arrivals.current.filter((t) => now - t < 5000);
      if (incoming.length) {
        if (pausedRef.current) {
          held.current.push(...incoming);
          if (held.current.length > MAX_ROWS) held.current = held.current.slice(-MAX_ROWS);
        } else {
          buffer.current = [...incoming.reverse(), ...buffer.current].slice(0, MAX_ROWS);
        }
        incoming = [];
      }
      setState((s) => ({
        ...s,
        rows: buffer.current,
        skipped: s.skipped + skipped,
        pendingWhilePaused: held.current.length,
        rate: arrivals.current.length / 5,
      }));
      skipped = 0;
    };
    // Batch renders: at most ~5 per second regardless of event rate.
    const timer = window.setInterval(flush, 200);
    es.addEventListener('ready', () => setState((s) => ({ ...s, connected: true, error: null })));
    es.addEventListener('request', (ev) => {
      try {
        incoming.push(JSON.parse((ev as MessageEvent<string>).data) as LiveRequest);
        arrivals.current.push(Date.now());
      } catch {
        // ignore malformed event
      }
    });
    es.addEventListener('skipped', (ev) => {
      try {
        skipped += (JSON.parse((ev as MessageEvent<string>).data) as { count: number }).count;
      } catch {
        // ignore
      }
    });
    es.onerror = () => setState((s) => ({ ...s, connected: false, error: 'reconnecting…' }));
    return () => {
      window.clearInterval(timer);
      es.close();
    };
  }, [project, filter, mode, environment, enabled]);

  return state;
}
