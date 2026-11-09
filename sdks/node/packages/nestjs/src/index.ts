export { TrailscopeModule } from './module.js';
export type { TrailscopeModuleAsyncOptions } from './module.js';
export { TrailscopeExceptionFilter, TrailscopeInterceptor, TrailscopeMiddleware, contextOfRequest } from './http.js';
export { DEFAULT_IGNORED_LOGGER_CONTEXTS, TrailscopeLogger } from './logger.js';
export { TRAILSCOPE_JOB_KEY, TrackJob, trackJob, withTrailscopeContext } from './jobs.js';
export type { JobLike, TrackJobOptions } from './jobs.js';
export { TRAILSCOPE_CLIENT, TRAILSCOPE_MODULE_OPTIONS } from './tokens.js';
export type { TrailscopeModuleOptions } from './tokens.js';
export {
  TrailscopeClient,
  currentContext,
  getClient,
  recordError,
  setAttribute,
  setCorrelation,
  setUser,
} from '@trailscope/node';
export type { TrailscopeContext, TrailscopeOptions } from '@trailscope/node';
