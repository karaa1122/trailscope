import { InjectQueue } from '@nestjs/bullmq';
import {
  Body,
  CanActivate,
  Controller,
  ExecutionContext,
  ForbiddenException,
  Get,
  Logger,
  Param,
  Post,
  UseGuards,
} from '@nestjs/common';
import { setCorrelation, setUser, withLogboxContext } from '@logbox/nestjs';
import type { Queue } from 'bullmq';

/** Pretend authentication: attach the caller to everything this request does. */
class Authenticated implements CanActivate {
  canActivate(context: ExecutionContext): boolean {
    const user = context.switchToHttp().getRequest<{ headers: Record<string, string | undefined> }>().headers['x-user-id'];
    if (user) setUser(user);
    return true;
  }
}

class AdminOnly implements CanActivate {
  canActivate(): boolean {
    throw new ForbiddenException('admins only');
  }
}

@Controller('projects/:projectId')
@UseGuards(Authenticated)
export class TasksController {
  private readonly logger = new Logger(TasksController.name);

  constructor(@InjectQueue('notifications') private readonly notifications: Queue) {}

  @Get('tasks')
  list(@Param('projectId') projectId: string) {
    setCorrelation('project_id', projectId);
    this.logger.log(`listing tasks of ${projectId}`);
    return [{ id: 't1', title: 'Write the brief', done: false }];
  }

  @Post('tasks')
  async create(@Param('projectId') projectId: string, @Body() body: { title?: string }) {
    setCorrelation('project_id', projectId);
    const task = { id: `t${Date.now()}`, title: body.title ?? 'Untitled' };
    // The job carries this request's context, so its run is linked to it.
    // A task titled "flaky" fails its first notification attempt, to show retries.
    const data = withLogboxContext({ taskId: task.id, flaky: task.title === 'flaky' });
    await this.notifications.add('task.notify', data, { attempts: 2 });
    return task;
  }

  @Get('export')
  export(): never {
    throw new TypeError("Cannot read properties of undefined (reading 'columns')");
  }

  @Get('settings')
  @UseGuards(AdminOnly)
  settings() {
    return {};
  }
}
