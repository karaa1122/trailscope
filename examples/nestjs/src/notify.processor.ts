import { Processor, WorkerHost } from '@nestjs/bullmq';
import { Logger } from '@nestjs/common';
import { TrackJob } from '@trailscope/nestjs';
import type { Job } from 'bullmq';

@Processor('notifications')
export class NotifyProcessor extends WorkerHost {
  private readonly logger = new Logger(NotifyProcessor.name);

  // Recorded as a `kind=job` event with its attempt, linked to the request that queued it.
  @TrackJob()
  async process(job: Job<{ taskId: string; flaky: boolean }>): Promise<void> {
    await new Promise((r) => setTimeout(r, 40));
    if (job.data.flaky && job.attemptsMade === 0) throw new Error('mail server busy');
    this.logger.log(`notified watchers of ${job.data.taskId}`);
  }
}
