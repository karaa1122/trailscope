import { useQuery } from '@tanstack/react-query';
import { Navigate, Outlet, createRootRoute, createRoute, createRouter } from '@tanstack/react-router';

import { Shell } from './components/Shell';
import { Skeleton } from './components/ui';
import { api } from './lib/api';
import { parseQuerySearch } from './lib/query';
import { useSession } from './lib/session';
import { InviteAccept, Login, Setup } from './pages/Auth';
import { Explore, type ExploreSearch } from './pages/Explore';
import { Inspector, type InspectorSearch } from './pages/Inspector';
import { LiveTail, type LiveSearch } from './pages/LiveTail';
import { Placeholder } from './pages/Placeholder';

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

export interface JourneySearch {
  q?: string;
  /// The request the user came from (highlighted as "you came from here").
  came?: string;
}

const journeysRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/journeys',
  validateSearch: (raw: Record<string, unknown>): JourneySearch => ({ q: strParam(raw.q), came: strParam(raw.came) }),
  component: () => <Placeholder title="Journeys" note="Journeys arrive in the next milestone." />,
});

const errorsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/errors',
  component: () => <Placeholder title="Errors" note="The error groups screen arrives in the next milestone." />,
});

const errorRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/errors/$fingerprint',
  component: () => <Placeholder title="Error group" note="Error group details arrive in the next milestone." />,
});

const routesRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/routes',
  component: () => <Placeholder title="Routes" note="The full-page route heatmap arrives in the next milestone." />,
});

const alertsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/alerts',
  component: () => <Placeholder title="Alerts" note="Alerts arrive in a later milestone." />,
});

const viewsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/views',
  component: () => <Placeholder title="Saved views" note="Saved views get their own screen in the next milestone." />,
});

const settingsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: '/settings',
  component: () => <Placeholder title="Settings" note="Settings arrive in the next milestone." />,
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
