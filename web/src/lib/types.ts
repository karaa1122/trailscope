// Types mirroring the Logbox API (see /api/openapi.json).

export type Role = 'viewer' | 'member' | 'admin' | 'owner';

export interface User {
  id: string;
  email: string;
  name: string;
  role: Role;
  can_view_bodies: boolean;
}

export interface Me {
  user: User;
  csrfToken: string | null;
  oidc: { enabled: boolean; displayName: string };
}

export interface ProjectSettings {
  maskIp: boolean;
  bodyCapture: 'errors' | 'always' | 'never';
  slowMs: number;
  sampleRate: number;
  routeSampleRates: Record<string, number>;
  ignoreRoutes: string[];
  maxBodyBytes: number;
}

export interface Project {
  id: string;
  slug: string;
  name: string;
  settings: ProjectSettings;
  createdAt: string;
}

export type TableName = 'requests' | 'events';
export type QueryMode = 'sql' | 'search' | 'filters';

export type ChipOp =
  | '='
  | '!='
  | '>'
  | '>='
  | '<'
  | '<='
  | 'in'
  | 'not_in'
  | 'contains'
  | 'not_contains'
  | 'like'
  | 'not_like'
  | 'is_empty'
  | 'is_not_empty';

export type ChipScalar = string | number | boolean;

export interface Chip {
  field: string;
  key?: string;
  op: ChipOp;
  value: ChipScalar | ChipScalar[] | null;
}

export type QueryInput =
  | { mode: 'sql'; text: string }
  | { mode: 'search'; text: string }
  | { mode: 'filters'; chips: Chip[] };

export interface QueryErrorPos {
  message: string;
  line: number;
  col: number;
  len: number;
}

export interface ValidateResponse {
  ok: boolean;
  errors: QueryErrorPos[];
  chips: Chip[] | null;
  search: string | null;
  sql: string | null;
  conditions: number;
  estimatedRows: number | null;
}

export interface ColumnInfo {
  name: string;
  ch_type: string;
  kind: 'string' | 'number' | 'u64' | 'bool' | 'time' | 'map';
  description: string;
  filterable: boolean;
  facetable: boolean;
  groupable: boolean;
  sensitive: boolean;
  free_text: boolean;
  live: boolean;
}

export interface FunctionDoc {
  name: string;
  signature: string;
  description: string;
}

export interface Schema {
  tables: Record<TableName, ColumnInfo[]>;
  functions: FunctionDoc[];
  keywords: string[];
}

export type Row = Record<string, unknown>;

export interface RowsResponse {
  rows: Row[];
  nextCursor: string | null;
  elapsedMs: number;
  readRows: number;
}

export interface HistogramResponse {
  bucketMs: number;
  series: string[];
  points: number[][];
  total: number;
  elapsedMs: number;
}

export interface FacetValue {
  value: string;
  count: number;
}

export interface Facet {
  field: string;
  values: FacetValue[];
}

export interface GroupRow {
  value: string;
  count: number;
  errorRate: number;
  clientErrorRate: number | null;
  p50: number;
  p95: number;
  p99: number;
}

export interface Tile {
  value: number;
  previous: number;
  series: number[];
}

export interface Overview {
  minutes: number;
  times: number[];
  requestsPerMin: Tile;
  errorRate: Tile;
  p95: Tile;
  slow: Tile;
  slowMs: number;
}

export interface HeatCell {
  requests: number;
  errors: number;
  errorRate: number;
  p95: number;
}

export interface HeatRoute {
  route: string;
  services: string[];
  cells: (HeatCell | null)[];
  current: HeatCell;
}

export interface Heatmap {
  times: number[];
  routes: HeatRoute[];
  sloMs: number;
}

export interface ErrorGroup {
  fingerprint: string;
  errorType: string;
  message: string;
  service: string;
  route: string;
  source: string;
  environment: string;
  firstSeen: number;
  lastSeen: number;
  total: number;
  last15m: number;
  last24h: number;
  users: number;
  firstRelease: string;
  sampleRequestId: string;
  isNew: boolean;
  spiking: boolean;
  sparkline: number[];
}

export interface ServiceHealth {
  service: string;
  lastSeen: number;
  requests: number;
  errorRate: number;
  health: 'ok' | 'degraded' | 'failing' | 'idle';
}

export interface Release {
  id: string;
  environment: string;
  service: string;
  version: string;
  commitSha: string | null;
  url: string | null;
  at: number;
}

export interface RequestDetail {
  request: Row;
  spans: Row[];
  events: Row[];
  errorGroup: Row | null;
  bodiesHidden: boolean;
}

export interface ReplayTarget {
  id: string;
  name: string;
  baseUrl: string;
}

export interface ReplayResult {
  id: string;
  url: string;
  status: number | null;
  headers: Record<string, string>;
  body: string;
  bodyTruncated: boolean;
  durationMs: number;
  error: string | null;
}

/** A request summary as streamed by the live tail. */
export interface LiveRequest {
  projectId: string;
  environment: string;
  service: string;
  release: string;
  timestamp: number;
  requestId: string;
  traceId: string;
  method: string;
  route: string;
  path: string;
  query: string;
  status: number;
  durationMs: number;
  reqSize: number;
  resSize: number;
  userId: string;
  client: string;
  clientVersion: string;
  ip: string;
  userAgent: string;
  errorType: string;
  errorMessage: string;
  errorFingerprint: string;
  attributes: [string, string][];
  correlation: [string, string][];
  sampleRate: number;
}

export interface SavedView {
  id: string;
  project: string;
  ownerId: string | null;
  name: string;
  table: TableName;
  query: QueryInput;
  timeRange: { relative?: string; from?: number; to?: number };
  columns: string[];
  shared: boolean;
  updatedAt: string;
}

export type JourneyStatus = 'ok' | 'warn' | 'error';

export interface JourneyItem {
  t: number;
  kind: 'request' | 'job' | 'log' | 'custom' | string;
  lane: string;
  status: JourneyStatus | string;
  title: string;
  detail: string;
  durationMs: number;
  requestId: string;
  traceId: string;
  fingerprint: string;
  similar: number;
  until: number;
}

export interface Journey {
  key: string;
  value: string;
  summary: {
    first: number;
    last: number;
    requests: number;
    jobs: number;
    errors: number;
    warnings: number;
    services: string[];
    inProgress: boolean;
    collapsed: number;
  };
  lanes: string[];
  items: JourneyItem[];
  context: Record<string, Record<string, string>>;
  errors: { fingerprint: string; errorType: string; count: number; status: string }[];
  traces: { traceId: string; spans: number; services: string[] }[];
  truncated: boolean;
}

export interface RouteSeries {
  route: string;
  bucketMinutes: number;
  times: number[];
  requests: number[];
  errors5xx: number[];
  errors4xx: number[];
  p50: number[];
  p95: number[];
  p99: number[];
  methods: Record<string, number>;
  sloMs: number;
}

export interface ErrorDetail {
  group: ErrorGroup;
  samples: Row[];
  hourly: [number, number][];
  stack: string;
}

export interface IngestKey {
  id: string;
  name: string;
  prefix: string;
  createdAt: string;
  lastUsedAt: string | null;
  revokedAt: string | null;
}

export interface RedactionRule {
  id: string;
  kind: 'header' | 'body_key' | 'regex';
  pattern: string;
}

export interface Member {
  id: string;
  email: string;
  name: string;
  role: Role;
  canViewBodies: boolean;
  disabled: boolean;
  createdAt: string;
  lastLoginAt: string | null;
}

export interface Invite {
  id: string;
  email: string;
  role: Role;
  canViewBodies: boolean;
  expiresAt: string;
}

export interface ApiToken {
  id: string;
  name: string;
  prefix: string;
  scopes: string[];
  projectId: string | null;
  createdAt: string;
  lastUsedAt: string | null;
  expiresAt: string | null;
}
