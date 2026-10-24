//! Saved views: a named query (in whichever mode it was written), time range
//! and columns, private or shared with everyone.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use logbox_query::{Ctx, QueryInput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::types::Json as SqlJson;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::auth::{Auth, Role, Scope};
use crate::error::{ApiError, ApiResult};
use crate::query::TableName;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: Uuid,
    pub project: Uuid,
    pub owner_id: Option<Uuid>,
    pub name: String,
    pub table: TableName,
    #[schema(value_type = Object)]
    pub query: QueryInput,
    /// `{"relative": "15m"}` or `{"from": ms, "to": ms}`.
    #[schema(value_type = Object)]
    pub time_range: Value,
    pub columns: Vec<String>,
    pub shared: bool,
    #[schema(value_type = String)]
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: time::OffsetDateTime,
}

#[derive(sqlx::FromRow)]
struct ViewRow {
    id: Uuid,
    project_id: Uuid,
    owner_id: Option<Uuid>,
    name: String,
    query: SqlJson<Value>,
    time_range: SqlJson<Value>,
    columns: SqlJson<Vec<String>>,
    shared: bool,
    updated_at: time::OffsetDateTime,
}

impl ViewRow {
    fn into_view(self) -> Option<View> {
        let q = self.query.0;
        let table: TableName =
            serde_json::from_value(q.get("table").cloned().unwrap_or(json!("requests"))).ok()?;
        let query: QueryInput = serde_json::from_value(q.get("input").cloned()?).ok()?;
        Some(View {
            id: self.id,
            project: self.project_id,
            owner_id: self.owner_id,
            name: self.name,
            table,
            query,
            time_range: self.time_range.0,
            columns: self.columns.0,
            shared: self.shared,
            updated_at: self.updated_at,
        })
    }
}

const COLUMNS: &str =
    "id, project_id, owner_id, name, query, time_range, columns, shared, updated_at";

#[derive(Deserialize, utoipa::IntoParams)]
pub struct ProjectParam {
    pub project: Uuid,
}

#[utoipa::path(get, path = "/api/v1/views", tag = "views", params(ProjectParam), responses((status = 200, body = Vec<View>)))]
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<ProjectParam>,
) -> ApiResult<Json<Vec<View>>> {
    auth.require(Role::Viewer, Scope::Read)?;
    auth.check_project(q.project)?;
    let rows: Vec<ViewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM saved_views WHERE project_id = $1 AND (shared OR owner_id = $2) ORDER BY name"
    )))
    .bind(q.project)
    .bind(auth.user.id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        rows.into_iter().filter_map(ViewRow::into_view).collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewInput {
    pub project: Uuid,
    pub name: String,
    #[serde(default)]
    pub table: TableName,
    #[schema(value_type = Object)]
    pub query: QueryInput,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub time_range: Value,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub shared: bool,
}

fn check(auth: &Auth, v: &ViewInput) -> ApiResult<()> {
    let name = v.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(ApiError::bad_request(
            "give the view a name (up to 120 characters)",
        ));
    }
    if v.columns.len() > 64 {
        return Err(ApiError::bad_request("too many columns"));
    }
    let ctx = Ctx {
        table: v.table.into(),
        allow_sensitive: auth.can_view_bodies(),
        live: false,
    };
    v.query.to_expr(&ctx).map_err(ApiError::invalid_query)?;
    Ok(())
}

#[utoipa::path(post, path = "/api/v1/views", tag = "views", request_body = ViewInput, responses((status = 200, body = View)))]
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(v): Json<ViewInput>,
) -> ApiResult<Json<View>> {
    auth.require(Role::Viewer, Scope::Write)?;
    auth.check_project(v.project)?;
    state.project_key(v.project).await?;
    check(&auth, &v)?;
    if v.shared && auth.user.role < Role::Member {
        return Err(ApiError::forbidden("viewers can only save private views"));
    }
    let id = Uuid::now_v7();
    let row: ViewRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO saved_views (id, project_id, owner_id, name, query, time_range, columns, shared) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING {COLUMNS}"
    )))
    .bind(id)
    .bind(v.project)
    .bind(auth.user.id)
    .bind(v.name.trim())
    .bind(SqlJson(json!({ "table": v.table, "input": v.query })))
    .bind(SqlJson(v.time_range))
    .bind(SqlJson(v.columns))
    .bind(v.shared)
    .fetch_one(&state.pool)
    .await?;
    row.into_view()
        .map(Json)
        .ok_or_else(|| ApiError::internal("stored view didn't round-trip"))
}

async fn owned(state: &AppState, auth: &Auth, id: Uuid) -> ApiResult<ViewRow> {
    let row: ViewRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM saved_views WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("view"))?;
    auth.check_project(row.project_id)?;
    let mine = row.owner_id == Some(auth.user.id);
    if !mine && !(row.shared && auth.user.role >= Role::Admin) {
        return Err(if row.shared {
            ApiError::forbidden("only its owner or an admin can change this view")
        } else {
            ApiError::not_found("view")
        });
    }
    Ok(row)
}

#[utoipa::path(put, path = "/api/v1/views/{id}", tag = "views", request_body = ViewInput, responses((status = 200, body = View)))]
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(v): Json<ViewInput>,
) -> ApiResult<Json<View>> {
    auth.require(Role::Viewer, Scope::Write)?;
    let existing = owned(&state, &auth, id).await?;
    if existing.project_id != v.project {
        return Err(ApiError::bad_request("views can't move between projects"));
    }
    check(&auth, &v)?;
    if v.shared && auth.user.role < Role::Member {
        return Err(ApiError::forbidden("viewers can only save private views"));
    }
    sqlx::query(
        "UPDATE saved_views SET name = $2, query = $3, time_range = $4, columns = $5, shared = $6, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(v.name.trim())
    .bind(SqlJson(json!({ "table": v.table, "input": v.query })))
    .bind(SqlJson(v.time_range))
    .bind(SqlJson(v.columns))
    .bind(v.shared)
    .execute(&state.pool)
    .await?;
    let row = owned(&state, &auth, id).await?;
    row.into_view()
        .map(Json)
        .ok_or_else(|| ApiError::internal("stored view didn't round-trip"))
}

#[utoipa::path(delete, path = "/api/v1/views/{id}", tag = "views", responses((status = 204)))]
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    auth.require(Role::Viewer, Scope::Write)?;
    owned(&state, &auth, id).await?;
    sqlx::query("DELETE FROM saved_views WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
