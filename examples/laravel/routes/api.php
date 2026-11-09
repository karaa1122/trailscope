<?php

use App\Http\Controllers\TaskController;
use App\Http\Middleware\PretendAuthentication;
use Illuminate\Support\Facades\Route;

Route::get('/health', fn () => 'ok'); // ignored by default

Route::middleware(PretendAuthentication::class)->group(function () {
    Route::get('/projects/{projectId}/tasks', [TaskController::class, 'index']);
    Route::post('/projects/{projectId}/tasks', [TaskController::class, 'store']);
    Route::get('/projects/{projectId}/export', [TaskController::class, 'export']);
});
