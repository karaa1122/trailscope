//! Plans and usage on the API side (decision 071): the project and member caps
//! (enforced only with `cloud.enabled`) and the usage numbers shown in the
//! admin console and in Settings → Organization.

use std::collections::HashMap;

use serde::Serialize;
use trailscope_query::compile::{ParamValue, Sql};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use crate::error::{ApiError, ApiResult};
use crate::query::{fetch_json, param};

/// SQL for the first day of the current month (UTC), as a `DATE`.
pub const MONTH_START: &str = "date_trunc('month', now() AT TIME ZONE 'UTC')::date";

#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub name: String,
    /// Monthly price in Iraqi dinars.
    pub price_iqd: i64,
    /// Requests, job runs and log lines per month; `null` = unlimited.
    pub events_per_month: Option<i64>,
    /// Stop accepting at the limit (otherwise the excess is overage).
    pub hard_limit: bool,
    pub max_projects: Option<i32>,
    pub max_members: Option<i32>,
    pub archived: bool,
    pub position: i32,
}

pub const PLAN_COLUMNS: &str = "p.id, p.name, p.price_iqd, p.events_per_month, p.hard_limit, \
     p.max_projects, p.max_members, p.archived, p.position";

/// The organization's plan.
pub async fn org_plan(state: &AppState, org: Uuid) -> ApiResult<Plan> {
    let sql = format!(
        "SELECT {PLAN_COLUMNS} FROM organizations o JOIN plans p ON p.id = o.plan_id WHERE o.id = $1"
    );
    sqlx::query_as::<_, Plan>(sqlx::AssertSqlSafe(sql))
        .bind(org)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("organization"))
}

/// Accepted events this calendar month (UTC), from `usage_daily`.
pub async fn month_events(state: &AppState, org: Uuid) -> ApiResult<i64> {
    let sql = format!(
        "SELECT COALESCE(SUM(events), 0)::bigint FROM usage_daily WHERE org_id = $1 AND day >= {MONTH_START}"
    );
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(org)
        .fetch_one(&state.pool)
        .await?)
}

/// Refuses a new project when the plan's project cap is reached.
pub async fn check_project_cap(state: &AppState, org: Uuid) -> ApiResult<()> {
    if !state.config.cloud.enabled {
        return Ok(());
    }
    let plan = org_plan(state, org).await?;
    let Some(max) = plan.max_projects else {
        return Ok(());
    };
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE org_id = $1")
        .bind(org)
        .fetch_one(&state.pool)
        .await?;
    if n >= i64::from(max) {
        return Err(ApiError::forbidden(format!(
            "the {} plan allows {max} project{}; upgrade for more",
            plan.name,
            if max == 1 { "" } else { "s" }
        )));
    }
    Ok(())
}

/// Refuses a new invite when members plus open invites reach the plan's cap.
pub async fn check_member_cap(state: &AppState, org: Uuid) -> ApiResult<()> {
    if !state.config.cloud.enabled {
        return Ok(());
    }
    let plan = org_plan(state, org).await?;
    let Some(max) = plan.max_members else {
        return Ok(());
    };
    let n: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM memberships WHERE org_id = $1 AND NOT disabled) \
              + (SELECT count(*) FROM invites WHERE org_id = $1 AND accepted_at IS NULL AND expires_at > now())",
    )
    .bind(org)
    .fetch_one(&state.pool)
    .await?;
    if n >= i64::from(max) {
        return Err(ApiError::forbidden(format!(
            "the {} plan allows {max} members (open invites count too); upgrade for more",
            plan.name
        )));
    }
    Ok(())
}

/// Requests, job runs and log lines stored this month, per project, read from
/// ClickHouse. Best effort: an empty map when ClickHouse can't answer.
pub async fn project_month_events(state: &AppState, projects: &[Uuid]) -> HashMap<Uuid, i64> {
    if projects.is_empty() {
        return HashMap::new();
    }
    let ids: Vec<String> = projects.iter().map(Uuid::to_string).collect();
    let mut sql = Sql::default();
    let p = param(&mut sql, "ids", "Array(String)", ParamValue::Strs(ids));
    let text = format!(
        "SELECT project_id, sum(n) AS n FROM ( \
           SELECT project_id, countMerge(requests) AS n FROM route_minute \
           WHERE project_id IN {p} AND minute >= toStartOfMonth(now('UTC')) GROUP BY project_id \
           UNION ALL \
           SELECT project_id, count() AS n FROM events \
           WHERE project_id IN {p} AND timestamp >= toStartOfMonth(now('UTC')) GROUP BY project_id \
         ) GROUP BY project_id"
    );
    match fetch_json(state, &text, &sql.params).await {
        Ok(rows) => rows
            .rows
            .iter()
            .filter_map(|r| {
                let id = r.get("project_id")?.as_str()?.parse().ok()?;
                Some((id, crate::overview::num(r, "n") as i64))
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = ?e, "counting this month's events per project");
            HashMap::new()
        }
    }
}
