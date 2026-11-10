import * as Dropdown from '@radix-ui/react-dropdown-menu';
import { useQuery } from '@tanstack/react-query';
import { Link, Outlet, useNavigate, useRouterState } from '@tanstack/react-router';
import { useState, type ComponentType, type ReactNode } from 'react';

import { api } from '../lib/api';
import { fmtCompact, fmtPct } from '../lib/format';
import { useHotkeys } from '../lib/hotkeys';
import { useSession } from '../lib/session';
import { CommandPalette } from './CommandPalette';
import {
  IconAlert,
  IconBell,
  IconBookmark,
  IconChevron,
  IconGrid,
  IconJourney,
  IconLive,
  IconSearch,
  IconSettings,
  Logo,
} from './icons';

interface NavItem {
  to: string;
  label: string;
  icon: ComponentType<{ size?: number }>;
  hint?: string;
}

const NAV: NavItem[] = [
  { to: '/live', label: 'Live tail', icon: IconLive, hint: 'G L' },
  { to: '/explore', label: 'Explore', icon: IconSearch, hint: 'G E' },
  { to: '/errors', label: 'Errors', icon: IconAlert, hint: 'G X' },
  { to: '/journeys', label: 'Journeys', icon: IconJourney, hint: 'G J' },
  { to: '/routes', label: 'Routes', icon: IconGrid, hint: 'G R' },
  { to: '/alerts', label: 'Alerts', icon: IconBell, hint: 'G A' },
  { to: '/views', label: 'Saved views', icon: IconBookmark },
  { to: '/settings', label: 'Settings', icon: IconSettings, hint: 'G S' },
];

/** Pages that use the collapsed icon rail instead of the full sidebar. */
const RAIL_PREFIXES = ['/requests/', '/journeys', '/errors/'];

export function Shell() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const navigate = useNavigate();
  const [palette, setPalette] = useState(false);
  const rail = RAIL_PREFIXES.some((p) => pathname.startsWith(p));

  useHotkeys({
    'mod+k': () => setPalette((o) => !o),
    '/': () => window.dispatchEvent(new Event('trailscope:focus-query')),
    'g l': () => void navigate({ to: '/live' }),
    'g e': () => void navigate({ to: '/explore' }),
    'g j': () => void navigate({ to: '/journeys' }),
    'g x': () => void navigate({ to: '/errors' }),
    'g r': () => void navigate({ to: '/routes' }),
    'g a': () => void navigate({ to: '/alerts' }),
    'g s': () => void navigate({ to: '/settings' }),
  });

  return (
    <div className="app">
      {rail ? <Rail pathname={pathname} /> : <Sidebar pathname={pathname} />}
      <Outlet />
      <CommandPalette open={palette} onOpenChange={setPalette} />
    </div>
  );
}

function isActive(pathname: string, to: string): boolean {
  return pathname === to || pathname.startsWith(`${to}/`);
}

function Rail({ pathname }: { pathname: string }) {
  return (
    <nav aria-label="Main" className="rail">
      <Link to="/live" aria-label="Trailscope home" style={{ marginBottom: 10 }}>
        <Logo />
      </Link>
      {NAV.slice(0, 6).map((n) => (
        <Link
          key={n.to}
          to={n.to}
          aria-label={n.label}
          title={n.label}
          className={
            isActive(pathname, n.to) || (n.to === '/explore' && pathname.startsWith('/requests/'))
              ? 'active'
              : undefined
          }
        >
          <n.icon size={18} />
        </Link>
      ))}
    </nav>
  );
}

function Sidebar({ pathname }: { pathname: string }) {
  const { me, project, environment } = useSession();
  const services = useQuery({
    queryKey: ['services', project?.id, environment],
    queryFn: () => api.services(project?.id ?? '', environment),
    enabled: !!project,
    refetchInterval: 30_000,
  });
  const errors = useQuery({
    queryKey: ['errors', 'badge', project?.id, environment],
    queryFn: () => api.errors(project?.id ?? '', environment, 'new_or_spiking', 50),
    enabled: !!project,
    refetchInterval: 60_000,
  });
  const badge = errors.data?.groups.length ?? 0;

  return (
    <aside className="sidebar">
      <Link to="/live" className="logo">
        <Logo />
        <span className="logo-word">trailscope</span>
      </Link>

      <ProjectSwitcher serviceCount={services.data?.length ?? 0} />

      <PlanNotice />

      <nav aria-label="Main" className="nav">
        {NAV.map((n) => (
          <Link key={n.to} to={n.to} className={isActive(pathname, n.to) ? 'active' : undefined}>
            <n.icon />
            {n.label}
            {n.to === '/errors' && badge > 0 ? (
              <span className="nav-badge" aria-label={`${badge} new or spiking`}>
                {badge}
              </span>
            ) : n.hint ? (
              <span className="nav-hint">{n.hint}</span>
            ) : null}
          </Link>
        ))}
        {me?.instanceAdmin && (
          <Link to="/admin">
            <IconGrid />
            Admin console
          </Link>
        )}
      </nav>

      <section aria-label="Services" className="services">
        <h2 className="section-title">Services</h2>
        {services.data?.length === 0 && (
          <span className="dim" style={{ padding: '0 10px', fontSize: 12 }}>
            No traffic yet
          </span>
        )}
        {services.data?.map((s) => {
          const color =
            s.health === 'failing'
              ? 'var(--err)'
              : s.health === 'degraded'
                ? 'var(--warn)'
                : s.health === 'idle'
                  ? 'var(--faint)'
                  : 'var(--accent)';
          const note =
            s.health === 'failing' || s.health === 'degraded'
              ? `5xx ${fmtPct(s.errorRate)}`
              : s.health === 'idle'
                ? 'idle'
                : `${fmtCompact(s.requests / 15)}/m`;
          return (
            <Link
              key={s.service}
              to="/explore"
              search={{ q: `service:${s.service}`, mode: 'search' }}
              className="service"
            >
              <span
                className="dot"
                style={{ width: 7, height: 7, background: color }}
                aria-hidden="true"
              />
              <span className="mono">{s.service}</span>
              <span
                className="service-note"
                style={{
                  color:
                    s.health === 'failing'
                      ? 'var(--err-text)'
                      : s.health === 'degraded'
                        ? 'var(--warn)'
                        : undefined,
                }}
              >
                {note}
              </span>
            </Link>
          );
        })}
      </section>

      <div className="hints">
        <kbd>/</kbd> search · <kbd>J</kbd> <kbd>K</kbd> move · <kbd>↵</kbd> inspect ·{' '}
        <kbd>Space</kbd> pause · <kbd>⌘K</kbd> commands
      </div>
    </aside>
  );
}

/** On the Cloud: a warning when the organization is suspended or has used its
 * plan's events for the month. */
function PlanNotice() {
  const { me } = useSession();
  const usage = useQuery({
    queryKey: ['org-usage', me?.org?.id],
    queryFn: api.orgUsage,
    enabled: !!me?.cloud && !!me.org,
    refetchInterval: 5 * 60_000,
  });
  const u = usage.data;
  if (!u || !u.enforced) return null;
  const limit = u.plan.eventsPerMonth;
  const over = u.plan.hardLimit && limit !== null && u.eventsThisMonth >= limit;
  if (!u.suspended && !over) return null;
  return (
    <Link
      to="/settings"
      search={{ tab: 'organization' }}
      className="panel panel-pad"
      role="status"
      style={{ fontSize: 12.5, lineHeight: 1.45, borderColor: 'var(--err)', color: 'var(--text)' }}
    >
      {u.suspended
        ? 'This organization is suspended: new data is refused. Contact the Trailscope operator.'
        : `The ${u.plan.name} plan's ${fmtCompact(limit ?? 0)} events for this month are used: new data is refused until next month.`}
    </Link>
  );
}

function ProjectSwitcher({ serviceCount }: { serviceCount: number }) {
  const { me, projects, project, environment, setProject, setEnvironment, switchOrg } =
    useSession();
  const navigate = useNavigate();
  const orgs = me?.orgs ?? [];
  // The organization is only worth showing once there is a choice to make.
  const showOrgs = orgs.length > 1 || !!me?.instanceAdmin;
  const envs = useQuery({
    queryKey: ['environments', project?.id],
    queryFn: () => api.environments(project?.id ?? ''),
    enabled: !!project,
    staleTime: 60_000,
  });
  return (
    <Dropdown.Root>
      <Dropdown.Trigger asChild>
        <button
          type="button"
          className="btn"
          style={{
            justifyContent: 'flex-start',
            gap: 10,
            padding: '0 12px',
            borderColor: 'var(--line-mid)',
            width: '100%',
          }}
        >
          <span className="dot" style={{ background: 'var(--accent)' }} aria-hidden="true" />
          <span
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'flex-start',
              lineHeight: 1.25,
              minWidth: 0,
              flex: 1,
            }}
          >
            <span style={{ fontWeight: 500 }} className="ellipsis">
              {environment ?? 'all environments'}
            </span>
            <span className="dim ellipsis" style={{ fontSize: 11 }}>
              {orgs.length > 1 && me?.org ? `${me.org.name} · ` : ''}
              {project?.name ?? 'No project'}
            </span>
          </span>
          <span style={{ marginLeft: 'auto', display: 'flex' }} title={`${serviceCount} services`}>
            <IconChevron />
          </span>
        </button>
      </Dropdown.Trigger>
      <Dropdown.Portal>
        <Dropdown.Content className="popover" sideOffset={6} align="start">
          <MenuLabel>Environment</MenuLabel>
          <Dropdown.Item className="menu-item" onSelect={() => setEnvironment(null)}>
            all environments {environment === null && <span className="nav-hint">✓</span>}
          </Dropdown.Item>
          {envs.data?.map((e) => (
            <Dropdown.Item key={e} className="menu-item" onSelect={() => setEnvironment(e)}>
              {e} {environment === e && <span className="nav-hint">✓</span>}
            </Dropdown.Item>
          ))}
          <MenuLabel>Project</MenuLabel>
          {projects.map((p) => (
            <Dropdown.Item key={p.id} className="menu-item" onSelect={() => setProject(p.id)}>
              {p.name} {project?.id === p.id && <span className="nav-hint">✓</span>}
            </Dropdown.Item>
          ))}
          {showOrgs && (
            <>
              <MenuLabel>Organization</MenuLabel>
              {orgs.map((o) => (
                <Dropdown.Item
                  key={o.id}
                  className="menu-item"
                  onSelect={() => {
                    if (o.id !== me?.org?.id) void switchOrg(o.id);
                  }}
                >
                  {o.name} {me?.org?.id === o.id && <span className="nav-hint">✓</span>}
                </Dropdown.Item>
              ))}
              {me?.instanceAdmin && (
                <Dropdown.Item
                  className="menu-item"
                  onSelect={() =>
                    void navigate({ to: '/settings', search: { tab: 'organization' } })
                  }
                >
                  <span className="dim">New organization…</span>
                </Dropdown.Item>
              )}
            </>
          )}
        </Dropdown.Content>
      </Dropdown.Portal>
    </Dropdown.Root>
  );
}

function MenuLabel({ children }: { children: ReactNode }) {
  return (
    <Dropdown.Label className="section-title" style={{ padding: '10px 12px 4px', margin: 0 }}>
      {children}
    </Dropdown.Label>
  );
}
