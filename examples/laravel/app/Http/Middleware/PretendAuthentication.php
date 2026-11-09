<?php

namespace App\Http\Middleware;

use Closure;
use Illuminate\Auth\GenericUser;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\Auth;

/**
 * Pretend authentication: the x-user-id header becomes the authenticated
 * user. Trailscope picks up the authenticated user's ID by itself, so there's
 * no Trailscope::setUser() call here.
 */
class PretendAuthentication
{
    public function handle(Request $request, Closure $next): mixed
    {
        $id = $request->header('x-user-id');
        if (is_string($id) && $id !== '') {
            Auth::setUser(new GenericUser(['id' => $id]));
        }

        return $next($request);
    }
}
