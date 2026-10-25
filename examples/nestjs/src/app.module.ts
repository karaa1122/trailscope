import { BullModule } from '@nestjs/bullmq';
import { Module } from '@nestjs/common';
import { LogboxModule } from '@logbox/nestjs';

import { NotifyProcessor } from './notify.processor.js';
import { TasksController } from './tasks.controller.js';

const redis = new URL(process.env.REDIS_URL ?? 'redis://localhost:6379');

@Module({
  imports: [
    LogboxModule.forRoot({
      service: 'tasks-api-nest',
      endpoint: process.env.LOGBOX_ENDPOINT ?? 'http://localhost:8080',
      exceptionFilter: true,
    }),
    BullModule.forRoot({ connection: { host: redis.hostname, port: Number(redis.port || 6379) } }),
    BullModule.registerQueue({ name: 'notifications' }),
  ],
  controllers: [TasksController],
  providers: [NotifyProcessor],
})
export class AppModule {}
