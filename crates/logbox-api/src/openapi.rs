//! Route table with OpenAPI generation (utoipa), served at
//! `/api/openapi.json` with an interactive reference at `/api/docs`.

use axum::Router;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

use crate::{
    AppState, errors, inspector, journeys, live, overview, projects, query, setup, users, views,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Logbox API",
        description = "Query, inspect and administer Logbox. Authenticate with a session cookie \
                       (plus `X-CSRF-Token` on writes) or `Authorization: Bearer lbx_api_…`.",
        license(name = "Apache-2.0", identifier = "Apache-2.0")
    ),
    modifiers(&SecuritySchemes),
    tags(
        (name = "setup"), (name = "auth"), (name = "members"), (name = "tokens"),
        (name = "projects"), (name = "query"), (name = "inspector"), (name = "live"), (name = "views"),
        (name = "overview"), (name = "errors"), (name = "journeys")
    )
)]
struct ApiDoc;

struct SecuritySchemes;

impl utoipa::Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .bearer_format("lbx_api_…")
                    .build(),
            ),
        );
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(
                crate::auth::SESSION_COOKIE,
            ))),
        );
    }
}

pub fn routes() -> OpenApiRouter<AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(setup::status, setup::complete))
        .routes(routes!(users::login))
        .routes(routes!(users::logout))
        .routes(routes!(users::me))
        .routes(routes!(users::change_password))
        .routes(routes!(users::list_members))
        .routes(routes!(users::update_member, users::delete_member))
        .routes(routes!(users::create_invite, users::list_invites))
        .routes(routes!(users::delete_invite))
        .routes(routes!(users::accept_invite))
        .routes(routes!(users::list_tokens, users::create_token))
        .routes(routes!(users::revoke_token))
        .routes(routes!(projects::list, projects::create))
        .routes(routes!(projects::get, projects::update, projects::delete))
        .routes(routes!(projects::list_keys, projects::create_key))
        .routes(routes!(projects::revoke_key))
        .routes(routes!(projects::list_rules, projects::create_rule))
        .routes(routes!(projects::delete_rule))
        .routes(routes!(projects::list_targets, projects::create_target))
        .routes(routes!(projects::delete_target))
        .routes(routes!(projects::environments))
        .routes(routes!(projects::services))
        .routes(routes!(projects::releases))
        .routes(routes!(projects::retention))
        .routes(routes!(query::validate))
        .routes(routes!(query::rows))
        .routes(routes!(query::histogram))
        .routes(routes!(query::facets))
        .routes(routes!(query::group))
        .routes(routes!(query::schema))
        .routes(routes!(query::values))
        .routes(routes!(query::map_keys))
        .routes(routes!(inspector::detail))
        .routes(routes!(inspector::curl))
        .routes(routes!(inspector::replay))
        .routes(routes!(live::live))
        .routes(routes!(views::list, views::create))
        .routes(routes!(views::update, views::delete))
        .routes(routes!(overview::overview))
        .routes(routes!(overview::heatmap))
        .routes(routes!(overview::route_series))
        .routes(routes!(errors::list))
        .routes(routes!(errors::detail))
        .routes(routes!(journeys::journey))
}

/// `/api/openapi.json` and the `/api/docs` reference page.
pub fn docs(doc: utoipa::openapi::OpenApi) -> Router<AppState> {
    let json = doc.clone();
    Router::new()
        .route(
            "/api/openapi.json",
            axum::routing::get(move || {
                let json = json.clone();
                async move { axum::Json(json) }
            }),
        )
        .merge(Scalar::with_url("/api/docs", doc))
}

/// The OpenAPI document, for tooling (`logbox openapi` prints it).
pub fn document() -> utoipa::openapi::OpenApi {
    routes().split_for_parts().1
}
