import { useQuery } from '@tanstack/react-query';
import { Navigate, Outlet, createRootRoute, createRoute, createRouter } from '@tanstack/react-router';

import { Shell } from './components/Shell';
import { Skeleton } from './components/ui';
import { api } from './lib/api';
import { parseQuerySearch } from './lib/query';
import { useSession } from './lib/session';
import { Alerts, type AlertsTab } from './pages/Alerts';
import { InviteAccept, Login, Setup } from './pages/Auth';
import { ErrorGroupPage, Errors, type ErrorsSearch } from './pages/Errors';
import { Explore, type ExploreSearch } from './pages/Explore';
import { Inspector, type InspectorSearch } from './pages/Inspector';
import { Journeys, type JourneySearch } from './pages/Journeys';
import { LiveTail, type LiveSearch } from './pages/LiveTail';
import { Placeholder } from './pages/Placeholder';
import { Routes, type RoutesSearch } from './pages/Routes';
import { Settings, type SettingsTab } from './pages/Settings';
import { Views } from './pages/Views';

const numParam = (v: unknown): number | undefined => {
  const n = typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : NaN;
  return Number.isFinite(n) && n > 0 ? n : undefined;
};
const strParam = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);

const rootRoute = createRootRoute({ component: () => <Outlet /> });

const loginRoute = createRoute({ getParentRoute: () => rootRoute, path: '/login', component: Login });
const setupRoute = createRoute({ getParentRoute: () => rootRoute, path: '/setup', component: Setup });
const inviteRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/invite/$token',
  component: function InvitePage() {
    const { token } = inviteRoute.useParams();
    return <InviteAccept token={token} />;
  },
});

function Guard() {
  const { me, loading, project } = useSession();
  const setup = useQuery({ queryKey: ['setup'], queryFn: api.setupStatus, enabled: !loading && !me });
  if (loading || (!me && setup.isLoading)) {
    return (
      <div style={{ padding: 40, display: 'flex', flexDirection: 'column', gap: 12 }}>
        <Skeleton h={28} w={220} />
        <Skeleton h={180} />
      </div>
    );
  }
  if (!me) return <Navigate to={setup.data?.needsSetup ? '/setup' : '/login'} />;
  if (!project) {
    return <Placeholder title="No projects" note="Ask an admin to create a project, or create one in Settings." />;
  }
  return <Shell />;
}

const appRoute = createRoute({ getParentRoute: () => rootRoute, id: 'app', component: Guard });

const indexRoute = createRoute({ getParentRoute: () => appRoute, path: '/', component: () => <Navigate to="/live" /> });

const liveRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/live',
  validateSearch: (raw: Record<string, unknown>): LiveSearch => ({ ...parseQuerySearch(raw), window: strParam(raw.window) }),
  component: function LivePage() {
    return <LiveTail search={liveRoute.useSearch()} />;
  },
});

const exploreRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/explore',
  validateSearch: (raw: Record<string, unknown>): ExploreSearch => ({
    ...parseQuerySearch(raw),
    table: raw.table === 'events' ? 'events' : undefined,
    range: strParam(raw.range),
    from: numParam(raw.from),
    to: numParam(raw.to),
    view: raw.view === 'group' ? 'group' : raw.view === 'rows' ? 'rows' : undefined,
    group: strParam(raw.group),
    cols: strParam(raw.cols),
  }),
  component: function ExplorePage() {
    return <Explore search={exploreRoute.useSearch()} />;
  },
});

const requestRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/requests/$requestId',
  validateSearch: (raw: Record<string, unknown>): InspectorSearch => ({
    ...parseQuerySearch(raw),
    from: numParam(raw.from),
    to: numParam(raw.to),
  }),
  component: function RequestPage() {
    const { requestId } = requestRoute.useParams();
    return <Inspector key={requestId} requestId={requestId} search={requestRoute.useSearch()} />;
  },
});

const journeysRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/journeys',
  validateSearch: (raw: Record<string, unknown>): JourneySearch => ({ q: strParam(raw.q), came: strParam(raw.came), range: strParam(raw.range) }),
  component: function JourneysPage() {
    return <Journeys search={journeysRoute.useSearch()} />;
  },
});

const errorsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/errors',
  validateSearch: (raw: Record<string, unknown>): ErrorsSearch => ({
    status: raw.status === 'new' || raw.status === 'spiking' ? raw.status : undefined,
    sort: raw.sort === 'count' || raw.sort === 'new' ? raw.sort : undefined,
    range: strParam(raw.range),
    q: strParam(raw.q),
  }),
  component: function ErrorsPage() {
    return <Errors search={errorsRoute.useSearch()} />;
  },
});

const errorRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/errors/$fingerprint',
  component: function ErrorPage() {
    const { fingerprint } = errorRoute.useParams();
    return <ErrorGroupPage key={fingerprint} fingerprint={fingerprint} />;
  },
});

const routesRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/routes',
  validateSearch: (raw: Record<string, unknown>): RoutesSearch => ({
    route: strParam(raw.route),
    minutes: numParam(raw.minutes),
    metric: raw.metric === 'errors' || raw.metric === 'volume' || raw.metric === 'p95' ? raw.metric : undefined,
  }),
  component: function RoutesPage() {
    return <Routes search={routesRoute.useSearch()} />;
  },
});

const alertsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/alerts',
  validateSearch: (raw: Record<string, unknown>): { tab?: AlertsTab } => ({
    tab: raw.tab === 'rules' || raw.tab === 'email' || raw.tab === 'channels' || raw.tab === 'history' ? raw.tab : undefined,
  }),
  component: function AlertsPage() {
    return <Alerts tab={alertsRoute.useSearch().tab} />;
  },
});

const viewsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/views',
  component: Views,
});

const settingsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/settings',
  validateSearch: (raw: Record<string, unknown>): { tab?: SettingsTab } => ({ tab: strParam(raw.tab) as SettingsTab | undefined }),
  component: function SettingsPage() {
    return <Settings tab={settingsRoute.useSearch().tab} />;
  },
});

const routeTree = rootRoute.addChildren([
  loginRoute,
  setupRoute,
  inviteRoute,
  appRoute.addChildren([
    indexRoute,
    liveRoute,
    exploreRoute,
    requestRoute,
    journeysRoute,
    errorsRoute,
    errorRoute,
    routesRoute,
    alertsRoute,
    viewsRoute,
    settingsRoute,
  ]),
]);

export const router = createRouter({ routeTree, defaultPreload: 'intent' });

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router;
  }
}
