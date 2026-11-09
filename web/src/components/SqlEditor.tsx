// CodeMirror 6 editor for SQL WHERE filters. Loaded lazily so the editor
// only ships when someone opens SQL mode.
import { autocompletion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { sql, StandardSQL } from '@codemirror/lang-sql';
import { linter, setDiagnostics, type Diagnostic } from '@codemirror/lint';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers, placeholder as cmPlaceholder } from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { useEffect, useRef } from 'react';

import type { QueryErrorPos, Schema, TableName } from '../lib/types';

export interface SqlEditorProps {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  errors: QueryErrorPos[];
  schema: Schema | undefined;
  table: TableName;
  allowSensitive: boolean;
  live: boolean;
  fetchValues: (column: string, prefix: string) => Promise<string[]>;
  fetchKeys: (map: string) => Promise<string[]>;
  placeholder?: string;
}

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: 'var(--accent)', fontWeight: '600' },
  { tag: [tags.string, tags.special(tags.string)], color: 'var(--ok)' },
  { tag: [tags.number, tags.bool], color: 'var(--warn)' },
  { tag: [tags.operator, tags.punctuation, tags.bracket], color: 'var(--muted)' },
  { tag: tags.comment, color: 'var(--err-text)' },
]);

/** Converts a 1-based line/col (characters) into a document offset. */
function offsetOf(state: EditorState, line: number, col: number): number {
  const l = state.doc.line(Math.min(Math.max(1, line), state.doc.lines));
  return Math.min(l.from + Math.max(0, col - 1), l.to);
}

function diagnostics(state: EditorState, errors: QueryErrorPos[]): Diagnostic[] {
  return errors.map((e) => {
    const from = offsetOf(state, e.line, e.col);
    const to = Math.min(from + Math.max(1, e.len), state.doc.length);
    return { from, to: to > from ? to : Math.min(from + 1, state.doc.length), severity: 'error', message: e.message };
  });
}

export default function SqlEditor(props: SqlEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(props);
  latest.current = props;

  useEffect(() => {
    if (!host.current) return;
    const complete = async (ctx: CompletionContext): Promise<CompletionResult | null> => {
      const p = latest.current;
      const before = ctx.state.sliceDoc(Math.max(0, ctx.pos - 200), ctx.pos);
      const mapKey = /(attr|attributes|correlation)\[\s*'([^']*)$/i.exec(before);
      if (mapKey) {
        const keys = await p.fetchKeys(mapKey[1]!.toLowerCase().startsWith('corr') ? 'correlation' : 'attributes');
        return {
          from: ctx.pos - mapKey[2]!.length,
          options: keys.map((k) => ({ label: k, type: 'property', detail: 'key', apply: `${k}']` })),
        };
      }
      const value = /([a-z_]+)\s*(?:=|!=|<>|in\s*\(|not\s+in\s*\(|like|ilike|,)\s*'([^']*)$/i.exec(before);
      if (value) {
        const col = p.schema?.tables[p.table].find((c) => c.name === value[1]!.toLowerCase());
        if (col && col.kind === 'string') {
          const vals = await p.fetchValues(col.name, value[2]!);
          return {
            from: ctx.pos - value[2]!.length,
            options: vals.map((v) => ({ label: v, type: 'text', detail: 'top value', apply: `${v.replace(/'/g, "''")}'` })),
          };
        }
        return null;
      }
      const word = ctx.matchBefore(/[A-Za-z_][\w]*/);
      if (!word && !ctx.explicit) return null;
      const cols = (p.schema?.tables[p.table] ?? []).filter(
        (c) => c.filterable && (!c.sensitive || p.allowSensitive) && (!p.live || c.live),
      );
      const options = [
        ...cols.map((c) =>
          c.kind === 'map'
            ? { label: `${c.name === 'attributes' ? 'attr' : c.name}['`, type: 'property', detail: 'map', apply: `${c.name === 'attributes' ? 'attr' : c.name}['` }
            : { label: c.name, type: 'variable', detail: c.ch_type, info: c.description },
        ),
        ...(p.schema?.functions ?? []).map((f) => ({ label: f.name, type: 'function', detail: 'function', info: f.signature, apply: `${f.name}(` })),
        ...(p.schema?.keywords ?? []).map((k) => ({ label: k, type: 'keyword', detail: 'keyword' })),
      ];
      return { from: word ? word.from : ctx.pos, options, validFor: /^\w*$/ };
    };

    const state = EditorState.create({
      doc: props.value,
      extensions: [
        lineNumbers(),
        history(),
        sql({ dialect: StandardSQL, upperCaseKeywords: true }),
        syntaxHighlighting(highlight),
        autocompletion({ override: [complete], icons: true, activateOnTyping: true }),
        linter(null),
        cmPlaceholder(props.placeholder ?? "status >= 500 AND route LIKE '/api/%'"),
        EditorView.lineWrapping,
        keymap.of([
          { key: 'Mod-Enter', run: () => (latest.current.onRun(), true) },
          { key: 'Escape', run: (v) => (v.contentDOM.blur(), true) },
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) latest.current.onChange(u.state.doc.toString());
        }),
        EditorView.contentAttributes.of({ 'aria-label': 'SQL WHERE filter' }),
      ],
    });
    view.current = new EditorView({ state, parent: host.current });
    const focus = () => view.current?.focus();
    window.addEventListener('trailscope:focus-query', focus);
    return () => {
      window.removeEventListener('trailscope:focus-query', focus);
      view.current?.destroy();
      view.current = null;
    };
    // The editor is created once; later prop changes flow through `latest`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // External value changes (mode switch, quick filters, history).
  useEffect(() => {
    const v = view.current;
    if (v && v.state.doc.toString() !== props.value) {
      v.dispatch({ changes: { from: 0, to: v.state.doc.length, insert: props.value } });
    }
  }, [props.value]);

  useEffect(() => {
    const v = view.current;
    if (v) v.dispatch(setDiagnostics(v.state, diagnostics(v.state, props.errors)));
  }, [props.errors, props.value]);

  return <div ref={host} className="editor-host" />;
}
