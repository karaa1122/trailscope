//! Alert rules: a query, a condition and a window, evaluated every
//! `alerts.interval_secs`. Each rule is `ok` or `firing`; a state change sends
//! one notification per channel, with a cooldown against flapping.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use trailscope_core::Config;
use trailscope_core::ch::ClickHouse;
use trailscope_query::ast::{CmpOp, Expr, Literal, Operand};
use trailscope_query::{Ctx, ParamValue, QueryInput, Sql, Table, where_clause};
use uuid::Uuid;

use crate::channels::{self, ChannelKind};
use crate::email5xx::now_ms;
use crate::outbox::{NewNotification, Outbox};
use crate::render::{self, RuleEvent};
use crate::smtp::Email;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// Matching requests (or events) in the window.
    Count,
    /// Share of 5xx among matching requests, in percent.
    ErrorRate,
    /// p95 of `duration_ms`, in ms.
    P95,
    /// Error groups first seen in the window.
    NewError,
    /// Error groups spiking in the window.
    SpikingError,
}

impl RuleKind {
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(Value::String(s.to_string())).ok()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::ErrorRate => "error_rate",
            Self::P95 => "p95",
            Self::NewError => "new_error",
            Self::SpikingError => "spiking_error",
        }
    }

    pub fn metric(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::ErrorRate => "5xx rate (%)",
            Self::P95 => "p95 latency (ms)",
            Self::NewError => "new error groups",
            Self::SpikingError => "spiking error groups",
        }
    }

    /// Whether the rule's query filter applies (error group rules use only the environment).
    pub fn uses_query(self) -> bool {
        matches!(self, Self::Count | Self::ErrorRate | Self::P95)
    }
}

/// The stored query: `{"table": "requests", "input": QueryInput}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleQuery {
    #[serde(default = "requests")]
    pub table: String,
    pub input: Option<QueryInput>,
}

fn requests() -> String {
    "requests".into()
}

impl RuleQuery {
    pub fn table(&self) -> Table {
        if self.table == "events" {
            Table::Events
        } else {
            Table::Requests
        }
    }

    pub fn text(&self) -> String {
        match &self.input {
            Some(QueryInput::Sql { text } | QueryInput::Search { text }) => text.clone(),
            Some(QueryInput::Filters { chips }) => format!("{} filters", chips.len()),
            None => String::new(),
        }
    }

    /// Parses the filter as an admin-defined rule (no bodies or headers).
    pub fn expr(&self) -> Result<Option<Expr>, String> {
        let ctx = Ctx {
            table: self.table(),
            allow_sensitive: false,
            live: false,
        };
        match &self.input {
            Some(i) => i.to_expr(&ctx).map_err(|e| e.message),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RuleRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub kind: String,
    pub query: sqlx::types::Json<RuleQuery>,
    pub environment: String,
    pub threshold: f64,
    pub window_minutes: i32,
    pub cooldown_minutes: i32,
    pub channel_ids: Vec<Uuid>,
    pub state: String,
    pub state_changed_at: Option<time::OffsetDateTime>,
    pub last_notified_at: Option<time::OffsetDateTime>,
}

#[derive(clickhouse::Row, Deserialize)]
struct V {
    v: f64,
}

pub async fn run(
    config: Config,
    ch: ClickHouse,
    pool: PgPool,
    outbox: Outbox,
    shutdown: CancellationToken,
) {
    let mut tick = tokio::time::interval(Duration::from_secs(config.alerts.interval_secs.max(10)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = tick.tick() => {
                if let Err(e) = evaluate_all(&config, &ch, &pool, &outbox).await {
                    tracing::warn!(error = %e, "alert evaluation");
                }
            }
        }
    }
}

async fn evaluate_all(
    config: &Config,
    ch: &ClickHouse,
    pool: &PgPool,
    outbox: &Outbox,
) -> Result<(), sqlx::Error> {
    let rules: Vec<RuleRow> = sqlx::query_as(
        "SELECT id, project_id, name, kind, query, environment, threshold, window_minutes, cooldown_minutes, \
           channel_ids, state, state_changed_at, last_notified_at FROM alert_rules WHERE enabled",
    )
    .fetch_all(pool)
    .await?;
    for rule in rules {
        let started = std::time::Instant::now();
        let value = evaluate(ch, &rule).await;
        metrics::histogram!("trailscope_alert_eval_seconds")
            .record(started.elapsed().as_secs_f64());
        match value {
            Ok(v) => apply(config, pool, outbox, &rule, v).await?,
            Err(e) => {
                sqlx::query("UPDATE alert_rules SET last_error = $2, last_evaluated_at = now() WHERE id = $1")
                    .bind(rule.id)
                    .bind(&e)
                    .execute(pool)
                    .await?;
            }
        }
    }
    Ok(())
}

/// The rule's current value.
pub async fn evaluate(ch: &ClickHouse, rule: &RuleRow) -> Result<f64, String> {
    let kind = RuleKind::parse(&rule.kind).ok_or("unknown rule kind")?;
    let now = now_ms();
    let window = i64::from(rule.window_minutes.max(1));
    let sql = build_sql(kind, rule, now, window)?;
    let mut q = ch.reader.query(&sql.text);
    for p in &sql.params {
        q = match &p.value {
            ParamValue::Str(s) => q.param(&p.name, s),
            ParamValue::Int(i) => q.param(&p.name, i),
            ParamValue::UInt(u) => q.param(&p.name, u),
            ParamValue::Float(f) => q.param(&p.name, f),
            ParamValue::Strs(v) => q.param(&p.name, v),
        };
    }
    let v: V = q.fetch_one().await.map_err(|e| e.to_string())?;
    Ok(if v.v.is_finite() { v.v } else { 0.0 })
}

fn build_sql(kind: RuleKind, rule: &RuleRow, now: i64, window: i64) -> Result<Sql, String> {
    let project = rule.project_id.to_string();
    if kind.uses_query() {
        let table = rule.query.table();
        if table == Table::Events && kind != RuleKind::Count {
            return Err("error rate and p95 rules need the requests table".into());
        }
        let mut filter = rule.query.expr()?;
        if !rule.environment.is_empty() {
            filter = and(filter, env_condition(table, &rule.environment));
        }
        let w = where_clause(table, &project, now - window * 60_000, now, filter.as_ref());
        let (tbl, agg) = match (kind, table) {
            (RuleKind::Count, Table::Events) => ("events", "toFloat64(count())"),
            (RuleKind::Count, _) => (
                "requests",
                "toFloat64(sum(if(sample_rate > 0, 1 / sample_rate, 1)))",
            ),
            (RuleKind::ErrorRate, _) => (
                "requests",
                "if(count() = 0, 0, countIf(status >= 500) / count() * 100)",
            ),
            _ => (
                "requests",
                "if(count() = 0, 0, toFloat64(quantile(0.95)(duration_ms)))",
            ),
        };
        return Ok(Sql {
            text: format!("SELECT toFloat64({agg}) AS v FROM {tbl} WHERE {}", w.text),
            params: w.params,
        });
    }
    let params = vec![
        trailscope_query::Param {
            name: "a_pid".into(),
            ty: "String",
            value: ParamValue::Str(project),
        },
        trailscope_query::Param {
            name: "a_env".into(),
            ty: "String",
            value: ParamValue::Str(rule.environment.clone()),
        },
        trailscope_query::Param {
            name: "a_win".into(),
            ty: "Int64",
            value: ParamValue::Int(window),
        },
    ];
    let text = if kind == RuleKind::NewError {
        "SELECT toFloat64(count()) AS v FROM (SELECT error_fingerprint FROM error_groups \
           WHERE project_id = {a_pid:String} AND (empty({a_env:String}) OR environment = {a_env:String}) \
           GROUP BY error_fingerprint HAVING min(first_seen) >= now() - toIntervalMinute({a_win:Int64}))"
    } else {
        // Spiking: at least 10 in the window and 5× the average window of the previous 24 h.
        "SELECT toFloat64(count()) AS v FROM (SELECT error_fingerprint, \
             sumIf(count, minute >= now() - toIntervalMinute({a_win:Int64})) AS cur, \
             sumIf(count, minute < now() - toIntervalMinute({a_win:Int64})) * {a_win:Int64} / 1440 AS base \
           FROM error_minute WHERE project_id = {a_pid:String} AND (empty({a_env:String}) OR environment = {a_env:String}) \
             AND minute >= now() - INTERVAL 1 DAY - toIntervalMinute({a_win:Int64}) \
           GROUP BY error_fingerprint HAVING cur >= 10 AND cur > 5 * base)"
    };
    Ok(Sql {
        text: text.into(),
        params,
    })
}

fn env_condition(table: Table, env: &str) -> Option<Expr> {
    let col = table.column("environment")?;
    Some(Expr::Compare {
        left: Operand::Column(col),
        op: CmpOp::Eq,
        right: Operand::Literal(Literal::Str(env.to_string())),
    })
}

fn and(a: Option<Expr>, b: Option<Expr>) -> Option<Expr> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(x), Some(y)) => Some(Expr::And(vec![x, y])),
    }
}

/// Updates state and queues notifications on a state change.
async fn apply(
    config: &Config,
    pool: &PgPool,
    outbox: &Outbox,
    rule: &RuleRow,
    value: f64,
) -> Result<(), sqlx::Error> {
    let firing_now = value > rule.threshold;
    let was_firing = rule.state == "firing";
    let now = time::OffsetDateTime::now_utc();
    let mut notify: Option<bool> = None;
    if firing_now && !was_firing {
        let cooled = rule
            .last_notified_at
            .is_none_or(|t| now - t >= time::Duration::minutes(i64::from(rule.cooldown_minutes)));
        if cooled {
            notify = Some(true);
        }
    } else if !firing_now && was_firing {
        // Only announce recovery if the firing was announced.
        let announced = match (rule.last_notified_at, rule.state_changed_at) {
            (Some(n), Some(c)) => n >= c,
            _ => false,
        };
        if announced {
            notify = Some(false);
        }
    }
    let changed = firing_now != was_firing;
    sqlx::query(
        "UPDATE alert_rules SET last_value = $2, last_error = NULL, last_evaluated_at = now(), \
           state = $3, state_changed_at = CASE WHEN $4 THEN now() ELSE state_changed_at END, \
           last_notified_at = CASE WHEN $5 THEN now() ELSE last_notified_at END WHERE id = $1",
    )
    .bind(rule.id)
    .bind(value)
    .bind(if firing_now { "firing" } else { "ok" })
    .bind(changed)
    .bind(notify.is_some())
    .execute(pool)
    .await?;
    if let Some(firing) = notify {
        notify_channels(config, pool, outbox, rule, firing, value).await?;
    }
    Ok(())
}

/// Builds the UI link with the rule's query pre-filled.
pub fn rule_link(public_url: &str, rule: &RuleRow, now: i64) -> String {
    let base = public_url.trim_end_matches('/');
    let kind = RuleKind::parse(&rule.kind);
    match kind {
        Some(RuleKind::NewError) => return format!("{base}/errors?status=new"),
        Some(RuleKind::SpikingError) => return format!("{base}/errors?status=spiking"),
        _ => {}
    }
    let from = now - i64::from(rule.window_minutes) * 60_000;
    let Ok(mut u) = url::Url::parse(&format!("{base}/explore")) else {
        return format!("{base}/explore");
    };
    {
        let mut qp = u.query_pairs_mut();
        match &rule.query.input {
            Some(QueryInput::Sql { text }) => {
                qp.append_pair("mode", "sql").append_pair("q", text);
            }
            Some(QueryInput::Search { text }) => {
                qp.append_pair("mode", "search").append_pair("q", text);
            }
            Some(QueryInput::Filters { chips }) => {
                qp.append_pair("mode", "filters")
                    .append_pair("chips", &serde_json::to_string(chips).unwrap_or_default());
            }
            None => {}
        }
        qp.append_pair("from", &from.to_string())
            .append_pair("to", &now.to_string());
    }
    u.to_string()
}

pub async fn notify_channels(
    config: &Config,
    pool: &PgPool,
    outbox: &Outbox,
    rule: &RuleRow,
    firing: bool,
    value: f64,
) -> Result<(), sqlx::Error> {
    let project_name: String = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(rule.project_id)
        .fetch_optional(pool)
        .await?
        .unwrap_or_default();
    let now = now_ms();
    let kind = RuleKind::parse(&rule.kind).unwrap_or(RuleKind::Count);
    let msg = render::rule_message(&RuleEvent {
        rule_id: rule.id,
        rule_name: &rule.name,
        project_name: &project_name,
        firing,
        metric: kind.metric().to_string(),
        value,
        threshold: rule.threshold,
        window_minutes: rule.window_minutes,
        environment: &rule.environment,
        query: &if kind.uses_query() {
            rule.query.text()
        } else {
            String::new()
        },
        link: rule_link(&config.server.public_url, rule, now),
        at_ms: now,
    });
    let reason = if firing { "firing" } else { "resolved" };
    let mut items = Vec::new();
    for ch in channels::load(pool, &rule.channel_ids).await? {
        let Some(kind) = ChannelKind::parse(&ch.kind) else {
            continue;
        };
        let base = NewNotification {
            project_id: Some(rule.project_id),
            source: "rule",
            rule_id: Some(rule.id),
            rule_name: rule.name.clone(),
            channel_id: Some(ch.id),
            channel_kind: kind,
            target: ch.name.clone(),
            reason,
            fingerprint: String::new(),
            subject: msg.subject.clone(),
            payload: json!({ "text": msg.text, "html": msg.telegram, "body": msg.body }),
        };
        if kind == ChannelKind::Email {
            for to in channels::recipients(pool, &ch.config.member_ids, &ch.config.emails).await? {
                items.push(NewNotification {
                    target: to.clone(),
                    payload: serde_json::to_value(Email {
                        to,
                        subject: msg.subject.clone(),
                        text: msg.text.clone(),
                        html: msg.email_html.clone(),
                    })
                    .unwrap_or_default(),
                    ..base.clone()
                });
            }
        } else {
            items.push(base);
        }
    }
    outbox.enqueue(items).await
}
