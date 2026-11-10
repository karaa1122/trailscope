import type {
  AlertChannel,
  AlertRule,
  ApiToken,
  Chip,
  EmailRule,
  ErrorDetail,
  ErrorGroup,
  Facet,
  GroupRow,
  Heatmap,
  HistogramResponse,
  IngestKey,
  Invite,
  InviteInfo,
  Journey,
  Me,
  Member,
  Mute,
  NotificationRow,
  Org,
  Overview,
  Project,
  QueryInput,
  RedactionRule,
  Release,
  ReplayResult,
  ReplayTarget,
  RequestDetail,
  Role,
  RouteSeries,
  RowsResponse,
  SavedView,
  Schema,
  SendResult,
  ServiceHealth,
  SmtpSettings,
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
    const err = (data as { error?: { code?: string; message?: string; details?: unknown } } | null)
      ?.error;
    throw new ApiError(
      res.status,
      err?.code ?? 'error',
      err?.message ?? res.statusText,
      err?.details,
    );
  }
  return data as T;
}

const get = <T>(path: string) => request<T>('GET', path);
const post = <T>(path: string, body: unknown) => request<T>('POST', path, body);
const patch = <T>(path: string, body: unknown) => request<T>('PATCH', path, body);
const put = <T>(path: string, body: unknown) => request<T>('PUT', path, body);
const del = (path: string) => request<undefined>('DELETE', path);

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
  setupStatus: () =>
    get<{ needsSetup: boolean; version: string; oidc: { enabled: boolean; displayName: string } }>(
      '/api/v1/setup',
    ),
  setup: (body: {
    email: string;
    name: string;
    password: string;
    projectName: string;
    organizationName?: string;
  }) =>
    post<{
      user: User;
      projectId: string;
      projectSlug: string;
      ingestKey: string;
      csrfToken: string;
    }>('/api/v1/setup', body),
  login: (email: string, password: string) =>
    post<{ user: User; csrfToken: string }>('/api/v1/auth/login', { email, password }),
  logout: () => post<undefined>('/api/v1/auth/logout', {}),
  me: () => get<Me>('/api/v1/auth/me'),
  acceptInvite: (token: string, name: string, password: string) =>
    post<{ user: User; csrfToken: string }>('/api/v1/invites/accept', { token, name, password }),
  lookupInvite: (token: string) => get<InviteInfo>(`/api/v1/invites/lookup${qs({ token })}`),

  orgs: () => get<Org[]>('/api/v1/orgs'),
  createOrg: (name: string) => post<Org>('/api/v1/orgs', { name }),
  renameOrg: (name: string) => patch<undefined>('/api/v1/orgs/current', { name }),
  switchOrg: (id: string) => post<undefined>(`/api/v1/orgs/${id}/switch`, {}),

  projects: () => get<Project[]>('/api/v1/projects'),
  environments: (project: string) => get<string[]>(`/api/v1/projects/${project}/environments`),
  services: (project: string, environment?: string | null) =>
    get<ServiceHealth[]>(`/api/v1/projects/${project}/services${qs({ environment })}`),
  releases: (project: string, from: number, to: number, environment?: string | null) =>
    get<Release[]>(`/api/v1/projects/${project}/releases${qs({ from, to, environment })}`),
  replayTargets: (project: string) =>
    get<ReplayTarget[]>(`/api/v1/projects/${project}/replay-targets`),

  schema: () => get<Schema>('/api/v1/schema'),
  validate: (body: {
    table: TableName;
    query: QueryInput;
    live?: boolean;
    project?: string;
    from?: number;
    to?: number;
  }) => post<ValidateResponse>('/api/v1/query/validate', body),
  rows: (
    scope: Scope & {
      columns?: string[];
      limit?: number;
      cursor?: string | null;
      order?: 'asc' | 'desc';
    },
  ) => post<RowsResponse>('/api/v1/query/requests', scope),
  histogram: (scope: Scope & { buckets?: number }) =>
    post<HistogramResponse>('/api/v1/query/histogram', scope),
  facets: (scope: Scope & { fields: { column: string; key?: string }[]; limit?: number }) =>
    post<{ facets: Facet[]; elapsedMs: number }>('/api/v1/query/facets', scope),
  group: (scope: Scope & { by: { column: string; key?: string }; limit?: number }) =>
    post<{ field: string; groups: GroupRow[]; elapsedMs: number }>('/api/v1/query/group', scope),
  values: (
    project: string,
    column: string,
    prefix: string,
    table: TableName = 'requests',
    key?: string,
  ) =>
    get<{ values: { value: string; count: number }[] }>(
      `/api/v1/values${qs({ project, column, prefix, table, key })}`,
    ),
  mapKeys: (project: string, map: string, table: TableName = 'requests') =>
    get<{ values: { value: string; count: number }[] }>(
      `/api/v1/keys${qs({ project, map, table })}`,
    ),

  overview: (project: string, environment: string | null, minutes: number) =>
    get<Overview>(`/api/v1/overview${qs({ project, environment, minutes })}`),
  heatmap: (project: string, environment: string | null, minutes: number, limit = 12) =>
    get<Heatmap>(`/api/v1/routes/heatmap${qs({ project, environment, minutes, limit })}`),
  errors: (
    project: string,
    environment: string | null,
    status?: string,
    limit = 50,
    opts: { from?: number; sort?: string; q?: string } = {},
  ) =>
    get<{ groups: ErrorGroup[] }>(
      `/api/v1/errors${qs({ project, environment, status, limit, ...opts })}`,
    ),
  errorDetail: (project: string, fingerprint: string, environment: string | null) =>
    get<ErrorDetail>(
      `/api/v1/errors/${encodeURIComponent(fingerprint)}${qs({ project, environment })}`,
    ),
  routeSeries: (
    project: string,
    route: string,
    environment: string | null,
    minutes: number,
    service?: string,
  ) =>
    get<RouteSeries>(
      `/api/v1/routes/series${qs({ project, route, environment, minutes, service })}`,
    ),
  journey: (
    project: string,
    key: string,
    value: string,
    environment: string | null,
    from?: number,
    to?: number,
  ) => get<Journey>(`/api/v1/journeys${qs({ project, key, value, environment, from, to })}`),

  request: (project: string, id: string) =>
    get<RequestDetail>(`/api/v1/requests/${encodeURIComponent(id)}${qs({ project })}`),
  curl: (project: string, id: string, target?: string) =>
    get<{ command: string; placeholders: string[] }>(
      `/api/v1/requests/${encodeURIComponent(id)}/curl${qs({ project, target })}`,
    ),
  replay: (project: string, id: string, targetId: string, headers: Record<string, string> = {}) =>
    post<ReplayResult>(`/api/v1/requests/${encodeURIComponent(id)}/replay`, {
      project,
      targetId,
      headers,
    }),

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
  updateView: (id: string, view: Omit<SavedView, 'id' | 'ownerId' | 'updatedAt'>) =>
    put<SavedView>(`/api/v1/views/${id}`, view),
  deleteView: (id: string) => del(`/api/v1/views/${id}`),

  createProject: (name: string) =>
    post<{ project: Project; ingestKey: string }>('/api/v1/projects', { name }),
  updateProject: (id: string, body: { name?: string; settings?: Project['settings'] }) =>
    patch<Project>(`/api/v1/projects/${id}`, body),
  deleteProject: (id: string) => del(`/api/v1/projects/${id}`),
  keys: (project: string) => get<IngestKey[]>(`/api/v1/projects/${project}/keys`),
  createKey: (project: string, name: string) =>
    post<{ id: string; token: string; prefix: string }>(`/api/v1/projects/${project}/keys`, {
      name,
    }),
  revokeKey: (project: string, id: string) => del(`/api/v1/projects/${project}/keys/${id}`),
  rules: (project: string) => get<RedactionRule[]>(`/api/v1/projects/${project}/redaction-rules`),
  createRule: (project: string, kind: RedactionRule['kind'], pattern: string) =>
    post<RedactionRule>(`/api/v1/projects/${project}/redaction-rules`, { kind, pattern }),
  deleteRule: (project: string, id: string) =>
    del(`/api/v1/projects/${project}/redaction-rules/${id}`),
  createTarget: (project: string, name: string, baseUrl: string) =>
    post<ReplayTarget>(`/api/v1/projects/${project}/replay-targets`, { name, baseUrl }),
  deleteTarget: (project: string, id: string) =>
    del(`/api/v1/projects/${project}/replay-targets/${id}`),
  retention: () => get<{ days: number; bodyDays: number }>('/api/v1/settings/retention'),

  members: () => get<Member[]>('/api/v1/members'),
  updateMember: (
    id: string,
    body: Partial<Pick<Member, 'role' | 'canViewBodies' | 'disabled' | 'name'>>,
  ) => patch<Member>(`/api/v1/members/${id}`, body),
  deleteMember: (id: string) => del(`/api/v1/members/${id}`),
  invites: () => get<Invite[]>('/api/v1/invites'),
  createInvite: (email: string, role: Role, canViewBodies: boolean) =>
    post<{ id: string; url: string; token: string }>('/api/v1/invites', {
      email,
      role,
      canViewBodies,
    }),
  deleteInvite: (id: string) => del(`/api/v1/invites/${id}`),
  tokens: () => get<ApiToken[]>('/api/v1/tokens'),
  createToken: (body: {
    name: string;
    scopes: string[];
    projectId?: string | null;
    expiresInDays?: number | null;
  }) => post<{ id: string; token: string; prefix: string }>('/api/v1/tokens', body),
  revokeToken: (id: string) => del(`/api/v1/tokens/${id}`),

  alertRules: (project: string) => get<AlertRule[]>(`/api/v1/alerts/rules${qs({ project })}`),
  createAlertRule: (rule: RuleInput) => post<AlertRule>('/api/v1/alerts/rules', rule),
  updateAlertRule: (id: string, rule: RuleInput) =>
    put<AlertRule>(`/api/v1/alerts/rules/${id}`, rule),
  deleteAlertRule: (project: string, id: string) =>
    del(`/api/v1/alerts/rules/${id}${qs({ project })}`),
  previewAlertRule: (rule: RuleInput) =>
    post<{ value: number; firing: boolean }>('/api/v1/alerts/rules/preview', rule),
  channels: (project: string) => get<AlertChannel[]>(`/api/v1/alerts/channels${qs({ project })}`),
  createChannel: (body: {
    project: string;
    name: string;
    kind: AlertChannel['kind'];
    url?: string;
    botToken?: string;
    chatId?: string;
    memberIds?: string[];
    emails?: string[];
  }) =>
    post<{ channel: AlertChannel; signingSecret: string | null }>('/api/v1/alerts/channels', body),
  deleteChannel: (project: string, id: string) =>
    del(`/api/v1/alerts/channels/${id}${qs({ project })}`),
  testChannel: (project: string, id: string) =>
    post<SendResult>(`/api/v1/alerts/channels/${id}/test${qs({ project })}`, {}),
  emailRule: (project: string) => get<EmailRule>(`/api/v1/alerts/email-rule${qs({ project })}`),
  saveEmailRule: (body: {
    project: string;
    enabled: boolean;
    filter: string;
    memberIds: string[];
    emails: string[];
  }) => put<EmailRule>('/api/v1/alerts/email-rule', body),
  testEmailRule: (project: string) =>
    post<SendResult>(`/api/v1/alerts/email-rule/test${qs({ project })}`, {}),
  smtp: () => get<SmtpSettings>('/api/v1/settings/smtp'),
  saveSmtp: (body: {
    host: string;
    port: number;
    security: SmtpSettings['security'];
    username: string;
    from: string;
    fromName: string;
    password?: string | null;
  }) => put<SmtpSettings>('/api/v1/settings/smtp', body),
  testSmtp: (to: string) => post<SendResult>('/api/v1/settings/smtp/test', { to }),
  history: (project: string, status?: string, limit = 100) =>
    get<NotificationRow[]>(`/api/v1/alerts/history${qs({ project, status, limit })}`),
  mutes: (project: string) => get<Mute[]>(`/api/v1/alerts/mutes${qs({ project })}`),
  unmute: (project: string, fingerprint: string) =>
    del(`/api/v1/alerts/mutes/${encodeURIComponent(fingerprint)}${qs({ project })}`),
  changePassword: (currentPassword: string, newPassword: string) =>
    post<undefined>('/api/v1/auth/password', { currentPassword, newPassword }),
};

export interface RuleInput {
  project: string;
  name: string;
  kind: AlertRule['kind'];
  query: AlertRule['query'];
  environment: string;
  threshold: number;
  windowMinutes: number;
  cooldownMinutes: number;
  channelIds: string[];
  enabled: boolean;
}

/** Server-Sent Events URL for the live tail. */
export function liveUrl(
  project: string,
  filter: string,
  mode: 'sql' | 'search',
  environment: string | null,
): string {
  return `/api/v1/live${qs({ project, filter, mode, environment })}`;
}

export function inputFor(
  mode: 'sql' | 'search' | 'filters',
  text: string,
  chips: Chip[],
): QueryInput {
  if (mode === 'filters') return { mode, chips };
  return { mode, text };
}
