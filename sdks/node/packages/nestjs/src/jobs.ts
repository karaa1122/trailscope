import { type TrailscopeClient, deserializeContext, getClient, serializeContext } from '@trailscope/node';

/** Key under which the request context travels in job data. */
export const TRAILSCOPE_JOB_KEY = '__trailscope';

/** The parts of a BullMQ `Job` the helpers use. */
export interface JobLike {
  name: string;
  id?: string | undefined;
  queueName?: string;
  attemptsMade?: number;
  attemptsStarted?: number;
  // BullMQ types job data as `any` by default.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data?: any;
}

export interface TrackJobOptions {
  /** Event name. Default: the job's name. */
  name?: string;
  /** Default: the process-wide client from `init()` or `TrailscopeModule`. */
  client?: TrailscopeClient;
}

/**
 * Adds the current request context to job data, so the job run is linked to
 * the request that queued it:
 *
 * ```ts
 * await queue.add('invoice.send', withTrailscopeContext({ invoiceId }));
 * ```
 */
export function withTrailscopeContext<T extends object>(data: T): T {
  const ctx = serializeContext();
  return ctx ? { ...data, [TRAILSCOPE_JOB_KEY]: ctx } : data;
}

/**
 * Wraps a BullMQ processor: restores the context from job data and records
 * each run as a `kind=job` event with its duration, outcome and attempt.
 *
 * ```ts
 * new Worker('invoices', trackJob(async (job) => send(job.data)), { connection });
 * ```
 */
export function trackJob<J extends JobLike, R>(
  processor: (job: J) => Promise<R> | R,
  opts: TrackJobOptions = {},
): (job: J) => Promise<R> {
  return async (job: J) => {
    const client = opts.client ?? getClient();
    if (!client) return processor(job);
    const data: unknown = job.data;
    const carried = data && typeof data === 'object' ? (data as Record<string, unknown>)[TRAILSCOPE_JOB_KEY] : undefined;
    const attributes: Record<string, string> = {};
    if (job.queueName) attributes.queue = job.queueName;
    if (job.id) attributes.job_id = job.id;
    return client.trackJob(opts.name ?? job.name, () => processor(job), {
      attempt: job.attemptsStarted || (job.attemptsMade ?? 0) + 1,
      context: deserializeContext(carried),
      attributes,
    });
  };
}

/**
 * Method decorator for `@nestjs/bullmq` processors:
 *
 * ```ts
 * @Processor('invoices')
 * export class InvoiceProcessor extends WorkerHost {
 *   @TrackJob()
 *   async process(job: Job) { ... }
 * }
 * ```
 */
export function TrackJob(opts: TrackJobOptions = {}): MethodDecorator {
  return (_target, _key, descriptor) => {
    const original = descriptor.value as unknown;
    if (typeof original !== 'function') return;
    const wrapped = function (this: unknown, job: JobLike, ...rest: unknown[]): Promise<unknown> {
      return trackJob((j: JobLike) => (original as (...a: unknown[]) => unknown).call(this, j, ...rest), opts)(job);
    };
    (descriptor as PropertyDescriptor).value = wrapped;
  };
}
