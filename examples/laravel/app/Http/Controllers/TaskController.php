<?php

namespace App\Http\Controllers;

use App\Jobs\NotifyWatchers;
use Illuminate\Http\JsonResponse;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Log;
use Trailscope\Trailscope;
use TypeError;

class TaskController
{
    public function index(string $projectId): JsonResponse
    {
        // A journey key: the request and everything it triggers carry it.
        Trailscope::setCorrelation('project_id', $projectId);
        Log::info("listing tasks of {$projectId}");
        $pending = Trailscope::span('db', 'SELECT count(*) FROM jobs', fn () => DB::table('jobs')->count());

        return response()->json([
            'tasks' => [['id' => 't1', 'title' => 'Write the brief', 'done' => false]],
            'pending_notifications' => $pending,
        ]);
    }

    public function store(Request $request, string $projectId): JsonResponse
    {
        Trailscope::setCorrelation('project_id', $projectId);
        $task = ['id' => 't'.(int) (microtime(true) * 1000), ...$request->json()->all()];
        // The queued job carries this request's context, so its runs are linked to it.
        NotifyWatchers::dispatch($task['id'], flaky: ($task['title'] ?? null) === 'flaky');

        return response()->json($task, 201);
    }

    public function export(string $projectId): never
    {
        Trailscope::setCorrelation('project_id', $projectId);

        throw new TypeError('Cannot access offset of type string on null');
    }
}
