<?php

namespace App\Jobs;

use Illuminate\Contracts\Queue\ShouldQueue;
use Illuminate\Foundation\Queue\Queueable;
use Illuminate\Support\Facades\Log;
use RuntimeException;

class NotifyWatchers implements ShouldQueue
{
    use Queueable;

    public int $tries = 3;

    public int $backoff = 1;

    public function __construct(public string $taskId, public bool $flaky = false) {}

    public function handle(): void
    {
        // A task titled "flaky" fails its first attempt, to show a retry.
        if ($this->flaky && $this->attempts() === 1) {
            throw new RuntimeException('mail server busy');
        }
        Log::info("notified watchers of {$this->taskId}");
    }
}
