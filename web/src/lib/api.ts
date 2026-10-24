import type {
  Chip,
  ErrorGroup,
  Facet,
  GroupRow,
  Heatmap,
  HistogramResponse,
  Me,
  Overview,
  Project,
  QueryInput,
  Release,
  ReplayResult,
  ReplayTarget,
  RequestDetail,
  RowsResponse,
  SavedView,
  Schema,
  ServiceHealth,
  TableName,
  User,
  ValidateResponse,
} from './types';

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
    readonly details?: unknown,
  ) {
    super(message);
  }
}

let csrfToken: string | null = null;

export function setCsrf(token: string | null) {
  csrfToken = token;
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const headers: Record<string, string> = { accept: 'application/json' };
  if (body !== undefined) headers['content-type'] = 'application/json';
  if (method !== 'GET' && csrfToken) headers['x-csrf-token'] = csrfToken;
  const res = await fetch(path, {
    method,
    headers,
    credentials: 'same-origin',
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 204) return undefined as T;
  const text = await res.text();
  let data: unknown;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }
  if (!res.ok) {
    const err = (data as { error?: { code?: string; message?: string; details?: unknown } } | null)?.error;
    throw new ApiError(res.status, err?.code ?? 'error', err?.message ?? res.statusText, err?.details);
  }
  return data as T;
}

const get = <T>(path: string) => request<T>('GET', path);
const post = <T>(path: string, body: unknown) => request<T>('POST', path, body);

function qs(params: Record<string, string | number | undefined | null>): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '') p.set(k, String(v));
  }
  const s = p.toString();
  return s ? `?${s}` : '';
}

export interface Scope {
  project: string;
  table?: TableName;
  query?: QueryInput | null;
  from?: number;
  to?: number;
  environment?: string | null;
}

export const api = {
  setupStatus: () => get<{ needsSetup: boolean; version: string }>('/api/v1/setup'),
  setup: (body: { email: string; name: string; password: string; projectName: string }) =>
    post<{ user: User; projectId: string; projectSlug: string; ingestKey: string; csrfToken: string }>(
      '/api/v1/setup',
      body,
    ),
  login: (email: string, password: string) =>
    post<{ user: User; csrfToken: string }>('/api/v1/auth/login', { email, password }),
  logout: () => post<undefined>('/api/v1/auth/logout', {}),
  me: () => get<Me>('/api/v1/auth/me'),
  acceptInvite: (token: string, name: string, password: string) =>
    post<{ user: User; csrfToken: string }>('/api/v1/invites/accept', { token, name, password }),

  projects: () => get<Project[]>('/api/v1/projects'),
  environments: (project: string) => get<string[]>(`/api/v1/projects/${project}/environments`),
  services: (project: string, environment?: string | null) =>
    get<ServiceHealth[]>(`/api/v1/projects/${project}/services${qs({ environment })}`),
  releases: (project: string, from: number, to: number, environment?: string | null) =>
    get<Release[]>(`/api/v1/projects/${project}/releases${qs({ from, to, environment })}`),
  replayTargets: (project: string) => get<ReplayTarget[]>(`/api/v1/projects/${project}/replay-targets`),

  schema: () => get<Schema>('/api/v1/schema'),
  validate: (body: {
    table: TableName;
    query: QueryInput;
    live?: boolean;
    project?: string;
    from?: number;
    to?: number;
  }) => post<ValidateResponse>('/api/v1/query/validate', body),
  rows: (scope: Scope & { columns?: string[]; limit?: number; cursor?: string | null; order?: 'asc' | 'desc' }) =>
    post<RowsResponse>('/api/v1/query/requests', scope),
  histogram: (scope: Scope & { buckets?: number }) => post<HistogramResponse>('/api/v1/query/histogram', scope),
  facets: (scope: Scope & { fields: { column: string; key?: string }[]; limit?: number }) =>
    post<{ facets: Facet[]; elapsedMs: number }>('/api/v1/query/facets', scope),
  group: (scope: Scope & { by: { column: string; key?: string }; limit?: number }) =>
    post<{ field: string; groups: GroupRow[]; elapsedMs: number }>('/api/v1/query/group', scope),
  values: (project: string, column: string, prefix: string, table: TableName = 'requests', key?: string) =>
    get<{ values: { value: string; count: number }[] }>(
      `/api/v1/values${qs({ project, column, prefix, table, key })}`,
    ),
  mapKeys: (project: string, map: string, table: TableName = 'requests') =>
    get<{ values: { value: string; count: number }[] }>(`/api/v1/keys${qs({ project, map, table })}`),

  overview: (project: string, environment: string | null, minutes: number) =>
    get<Overview>(`/api/v1/overview${qs({ project, environment, minutes })}`),
  heatmap: (project: string, environment: string | null, minutes: number, limit = 12) =>
    get<Heatmap>(`/api/v1/routes/heatmap${qs({ project, environment, minutes, limit })}`),
  errors: (project: string, environment: string | null, status?: string, limit = 50) =>
    get<{ groups: ErrorGroup[] }>(`/api/v1/errors${qs({ project, environment, status, limit })}`),

  request: (project: string, id: string) =>
    get<RequestDetail>(`/api/v1/requests/${encodeURIComponent(id)}${qs({ project })}`),
  curl: (project: string, id: string, target?: string) =>
    get<{ command: string; placeholders: string[] }>(
      `/api/v1/requests/${encodeURIComponent(id)}/curl${qs({ project, target })}`,
    ),
  replay: (project: string, id: string, targetId: string, headers: Record<string, string> = {}) =>
    post<ReplayResult>(`/api/v1/requests/${encodeURIComponent(id)}/replay`, { project, targetId, headers }),

  views: (project: string) => get<SavedView[]>(`/api/v1/views${qs({ project })}`),
  createView: (view: {
    project: string;
    name: string;
    table: TableName;
    query: QueryInput;
    timeRange: SavedView['timeRange'];
    columns: string[];
    shared: boolean;
  }) => post<SavedView>('/api/v1/views', view),
};

/** Server-Sent Events URL for the live tail. */
export function liveUrl(project: string, filter: string, mode: 'sql' | 'search', environment: string | null): string {
  return `/api/v1/live${qs({ project, filter, mode, environment })}`;
}

export function inputFor(mode: 'sql' | 'search' | 'filters', text: string, chips: Chip[]): QueryInput {
  if (mode === 'filters') return { mode, chips };
  return { mode, text };
}
