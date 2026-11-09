// Minimal NestJS app wired to Trailscope, with a BullMQ queue. Start Trailscope and Redis, then:
//   pnpm build && TRAILSCOPE_INGEST_KEY=trs_ingest_… pnpm start      (PLATFORM=fastify for the Fastify adapter)
//   curl -X POST localhost:3000/projects/p1/tasks -H 'content-type: application/json' -d '{"title":"Plan"}'
import 'reflect-metadata';
import { NestFactory } from '@nestjs/core';
import { FastifyAdapter } from '@nestjs/platform-fastify';
import { TrailscopeLogger } from '@trailscope/nestjs';

import { AppModule } from './app.module.js';

const app =
  process.env.PLATFORM === 'fastify'
    ? await NestFactory.create(AppModule, new FastifyAdapter(), { bufferLogs: true })
    : await NestFactory.create(AppModule, { bufferLogs: true });
app.useLogger(app.get(TrailscopeLogger));
app.enableShutdownHooks();
await app.listen(Number(process.env.PORT ?? 3000));
