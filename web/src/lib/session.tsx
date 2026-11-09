import { useQuery, useQueryClient } from '@tanstack/react-query';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import { ApiError, api, setCsrf } from './api';
import type { Me, Project } from './types';

interface SessionValue {
  me: Me | null;
  loading: boolean;
  projects: Project[];
  project: Project | null;
  environment: string | null;
  setProject: (id: string) => void;
  setEnvironment: (env: string | null) => void;
  refresh: () => Promise<void>;
  signedIn: (csrf: string) => Promise<void>;
  signOut: () => Promise<void>;
  theme: 'dark' | 'light';
  toggleTheme: () => void;
}

const SessionContext = createContext<SessionValue | null>(null);

function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // storage disabled; preferences just won't persist
  }
}

export function SessionProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const meQuery = useQuery({
    queryKey: ['me'],
    queryFn: async () => {
      try {
        const me = await api.me();
        setCsrf(me.csrfToken);
        return me;
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) return null;
        throw e;
      }
    },
    staleTime: 60_000,
  });
  const me = meQuery.data ?? null;
  const projectsQuery = useQuery({
    queryKey: ['projects'],
    queryFn: api.projects,
    enabled: !!me,
    staleTime: 60_000,
  });
  const projects = useMemo(() => projectsQuery.data ?? [], [projectsQuery.data]);
  const [projectId, setProjectId] = useState<string | null>(() => stored('trailscope.project'));
  const [environment, setEnv] = useState<string | null>(() => stored('trailscope.environment'));
  const [theme, setTheme] = useState<'dark' | 'light'>(() => (stored('trailscope.theme') === 'light' ? 'light' : 'dark'));

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  const project = projects.find((p) => p.id === projectId) ?? projects[0] ?? null;

  const setProject = useCallback((id: string) => {
    setProjectId(id);
    store('trailscope.project', id);
    setEnv(null);
    store('trailscope.environment', null);
  }, []);

  const setEnvironment = useCallback((env: string | null) => {
    setEnv(env);
    store('trailscope.environment', env);
  }, []);

  const refresh = useCallback(async () => {
    await qc.invalidateQueries({ queryKey: ['me'] });
    await qc.invalidateQueries({ queryKey: ['projects'] });
  }, [qc]);

  const signedIn = useCallback(
    async (csrf: string) => {
      setCsrf(csrf);
      await qc.resetQueries();
    },
    [qc],
  );

  const signOut = useCallback(async () => {
    try {
      await api.logout();
    } finally {
      setCsrf(null);
      qc.setQueryData(['me'], null);
      qc.removeQueries({ predicate: (q) => q.queryKey[0] !== 'me' });
    }
  }, [qc]);

  const toggleTheme = useCallback(() => {
    setTheme((t) => {
      const next = t === 'dark' ? 'light' : 'dark';
      store('trailscope.theme', next);
      return next;
    });
  }, []);

  const value: SessionValue = {
    me,
    loading: meQuery.isLoading || (!!me && projectsQuery.isLoading),
    projects,
    project,
    environment,
    setProject,
    setEnvironment,
    refresh,
    signedIn,
    signOut,
    theme,
    toggleTheme,
  };
  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession(): SessionValue {
  const v = useContext(SessionContext);
  if (!v) throw new Error('useSession outside SessionProvider');
  return v;
}

/** The current project; components using it are only rendered once one exists. */
export function useProject(): Project {
  const { project } = useSession();
  if (!project) throw new Error('no project selected');
  return project;
}
