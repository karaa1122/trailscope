//! Message rendering. Emails are HTML (dark text on light, one column, readable
//! on a phone) plus a plain-text part. They never contain request or response
//! bodies, headers or IP addresses: only fields of [`ErrorSignal`].

use std::fmt::Write as _;

use time::OffsetDateTime;
use time::macros::format_description;
use trailscope_core::rows::ErrorSignal;

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

pub fn fmt_time(ms: i64) -> String {
    let fmt = format_description!("[year]-[month]-[day] [hour]:[minute]:[second] UTC");
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .ok()
        .and_then(|t| t.format(&fmt).ok())
        .unwrap_or_default()
}

fn fmt_ms(ms: f32) -> String {
    if ms >= 10_000.0 {
        format!("{:.1} s", ms / 1000.0)
    } else if ms >= 10.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{ms:.1} ms")
    }
}

/// The first `n` stack frames.
pub fn top_frames(stack: &str, n: usize) -> Vec<String> {
    let lines: Vec<&str> = stack
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let frames: Vec<String> = lines
        .iter()
        .filter(|l| l.starts_with("at ") || l.starts_with("File \""))
        .take(n)
        .map(|l| (*l).to_string())
        .collect();
    if frames.is_empty() {
        lines
            .iter()
            .skip(1)
            .take(n)
            .map(|l| (*l).to_string())
            .collect()
    } else {
        frames
    }
}

/// Why a 5xx email is sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    /// The first occurrence of the error group.
    First,
    /// It kept happening after the previous email.
    Digest {
        count: u64,
        users: u64,
        minutes: u32,
    },
    /// It came back after a quiet period.
    Regression { quiet_hours: u64 },
}

impl Reason {
    pub fn key(&self) -> &'static str {
        match self {
            Reason::First => "first",
            Reason::Digest { .. } => "digest",
            Reason::Regression { .. } => "regression",
        }
    }
}

pub struct ErrorEmail<'a> {
    pub sig: &'a ErrorSignal,
    pub project_name: &'a str,
    pub reason: &'a Reason,
    /// Occurrences and users affected so far (all time).
    pub totals: (u64, u64),
    pub public_url: &'a str,
    /// `(label, url)` for 1 hour, 24 hours and forever.
    pub mute_links: Vec<(&'static str, String)>,
}

pub struct Rendered {
    pub subject: String,
    pub text: String,
    pub html: String,
}

pub fn error_subject(sig: &ErrorSignal, reason: &Reason) -> String {
    let what = if sig.error_type.is_empty() {
        format!("HTTP {}", sig.status)
    } else {
        sig.error_type.clone()
    };
    let base = format!(
        "{} {} {} — {} · {}",
        sig.status, sig.method, sig.route, what, sig.service
    );
    let subject = match reason {
        Reason::First => format!("[Trailscope] {base}"),
        Reason::Digest { count, .. } => format!("[Trailscope] {base} ({count} more)"),
        Reason::Regression { .. } => format!("[Trailscope] Regression: {base}"),
    };
    // Header injection is impossible through lettre, but keep subjects single-line.
    subject.replace(['\r', '\n'], " ")
}

pub fn error_email(e: &ErrorEmail<'_>) -> Rendered {
    let sig = e.sig;
    let base = e.public_url.trim_end_matches('/');
    let inspector = format!("{base}/requests/{}", sig.request_id);
    let group = format!("{base}/errors/{}", sig.error_fingerprint);
    let headline = match e.reason {
        Reason::First => format!(
            "A new error started in {} ({}).",
            sig.service, sig.environment
        ),
        Reason::Digest {
            count,
            users,
            minutes,
        } => format!(
            "{count} more in the last {minutes} min, {users} user{} affected.",
            if *users == 1 { "" } else { "s" }
        ),
        Reason::Regression { quiet_hours } => {
            format!("This error is back after {quiet_hours} hours without occurrences.")
        }
    };
    let error_line = if sig.error_message.is_empty() {
        sig.error_type.clone()
    } else if sig.error_type.is_empty() {
        sig.error_message.clone()
    } else {
        format!("{}: {}", sig.error_type, sig.error_message)
    };
    let fields: Vec<(&str, String)> = vec![
        ("Time", fmt_time(sig.timestamp)),
        ("Environment", sig.environment.clone()),
        ("Service", sig.service.clone()),
        (
            "Release",
            if sig.release.is_empty() {
                "—".into()
            } else {
                sig.release.clone()
            },
        ),
        ("Request", format!("{} {}", sig.method, sig.route)),
        ("Status", sig.status.to_string()),
        ("Duration", fmt_ms(sig.duration_ms)),
        ("Request ID", sig.request_id.clone()),
        (
            "User ID",
            if sig.user_id.is_empty() {
                "—".into()
            } else {
                sig.user_id.clone()
            },
        ),
        ("Occurrences so far", e.totals.0.to_string()),
        ("Users affected", e.totals.1.to_string()),
    ];
    let frames = top_frames(&sig.error_stack, 5);
    let subject = error_subject(sig, e.reason);

    let mut text = format!("{headline}\n\n{error_line}\n\n");
    for (k, v) in &fields {
        let _ = writeln!(text, "{k}: {v}");
    }
    if !frames.is_empty() {
        text.push_str("\nStack (top frames):\n");
        for f in &frames {
            let _ = writeln!(text, "  {f}");
        }
    }
    let _ = write!(
        text,
        "\nOpen in the inspector: {inspector}\nError group: {group}\n"
    );
    if !e.mute_links.is_empty() {
        text.push_str("\nMute this error:\n");
        for (label, url) in &e.mute_links {
            let _ = writeln!(text, "  {label}: {url}");
        }
    }
    let _ = write!(
        text,
        "\nYou receive this because you are a recipient of the \"Email on 5xx\" rule in project {}.\n",
        e.project_name
    );

    let mut rows = String::new();
    for (k, v) in &fields {
        let _ = write!(
            rows,
            r#"<tr><td style="padding:6px 12px 6px 0;color:#5b6575;font-size:13px;white-space:nowrap;vertical-align:top">{}</td><td style="padding:6px 0;font-family:ui-monospace,Menlo,Consolas,monospace;font-size:13px;color:#11151b;word-break:break-all">{}</td></tr>"#,
            esc(k),
            esc(v)
        );
    }
    let stack_html = if frames.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p style="margin:20px 0 6px;font-size:13px;color:#5b6575">Stack (top frames)</p><pre style="margin:0;padding:12px;background:#f1f3f6;border-radius:8px;font-size:12px;line-height:1.6;color:#2a313c;white-space:pre-wrap;word-break:break-all">{}</pre>"#,
            esc(&frames.join("\n"))
        )
    };
    let mute_html = if e.mute_links.is_empty() {
        String::new()
    } else {
        let links: Vec<String> = e
            .mute_links
            .iter()
            .map(|(label, url)| {
                format!(
                    r#"<a href="{}" style="color:#3a4250">{}</a>"#,
                    esc(url),
                    esc(label)
                )
            })
            .collect();
        format!(
            r#"<p style="margin:20px 0 0;font-size:13px;color:#5b6575">Mute this error: {}</p>"#,
            links.join(" · ")
        )
    };
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title></head>
<body style="margin:0;padding:0;background:#f6f7f9;font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#11151b">
<div style="max-width:600px;margin:0 auto;padding:24px 16px">
<div style="background:#ffffff;border:1px solid #e1e4ea;border-radius:12px;padding:24px">
<p style="margin:0 0 4px;font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:#5b6575">Trailscope · {project}</p>
<h1 style="margin:0 0 8px;font-size:20px;line-height:1.3;color:#c0392b;font-family:ui-monospace,Menlo,Consolas,monospace;word-break:break-word">{status} {method} {route}</h1>
<p style="margin:0 0 12px;font-size:15px;line-height:1.5">{headline}</p>
<p style="margin:0 0 16px;padding:12px;background:#fdf0ee;border-radius:8px;font-family:ui-monospace,Menlo,Consolas,monospace;font-size:13px;line-height:1.5;color:#7a1f14;word-break:break-word">{error}</p>
<table role="presentation" style="border-collapse:collapse;width:100%">{rows}</table>
{stack}
<p style="margin:24px 0 0"><a href="{inspector}" style="display:inline-block;padding:12px 18px;background:#11151b;color:#ffffff;text-decoration:none;border-radius:8px;font-size:14px;font-weight:600">Open in the inspector</a>
&nbsp; <a href="{group}" style="font-size:14px;color:#3a4250">Error group</a></p>
{mute}
</div>
<p style="margin:16px 4px 0;font-size:12px;line-height:1.5;color:#667080">You receive this because you are a recipient of the “Email on 5xx” rule in project {project}. Change the recipients under Alerts in Trailscope.</p>
</div></body></html>"#,
        title = esc(&subject),
        project = esc(e.project_name),
        status = sig.status,
        method = esc(&sig.method),
        route = esc(&sig.route),
        headline = esc(&headline),
        error = esc(&error_line),
        rows = rows,
        stack = stack_html,
        inspector = esc(&inspector),
        group = esc(&group),
        mute = mute_html,
    );
    Rendered {
        subject,
        text,
        html,
    }
}

/// The single "notifications paused" email sent when a project hits its hourly cap.
pub fn paused_email(
    project_name: &str,
    cap: u32,
    resumes_at_ms: i64,
    public_url: &str,
) -> Rendered {
    let subject = format!("[Trailscope] Email notifications paused for {project_name}");
    let link = format!("{}/errors", public_url.trim_end_matches('/'));
    let body = format!(
        "{project_name} reached the limit of {cap} error emails this hour, so further 5xx emails are paused until {}. \
         Errors are still recorded; the next email after the pause will include what was suppressed as a digest.",
        fmt_time(resumes_at_ms)
    );
    let text = format!("{body}\n\nAll errors: {link}\n");
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;background:#f6f7f9;font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#11151b">
<div style="max-width:600px;margin:0 auto;padding:24px 16px"><div style="background:#fff;border:1px solid #e1e4ea;border-radius:12px;padding:24px">
<h1 style="margin:0 0 12px;font-size:18px">Email notifications paused</h1>
<p style="margin:0 0 16px;font-size:15px;line-height:1.5">{}</p>
<a href="{}" style="display:inline-block;padding:12px 18px;background:#11151b;color:#fff;text-decoration:none;border-radius:8px;font-size:14px;font-weight:600">See all errors</a>
</div></div></body></html>"#,
        esc(&body),
        esc(&link)
    );
    Rendered {
        subject,
        text,
        html,
    }
}

/// A rule notification in every channel format.
pub struct RuleMessage {
    pub subject: String,
    pub text: String,
    /// Telegram-flavoured HTML (b, a, code only).
    pub telegram: String,
    pub email_html: String,
    /// Webhook JSON body.
    pub body: serde_json::Value,
}

pub struct RuleEvent<'a> {
    pub rule_id: uuid::Uuid,
    pub rule_name: &'a str,
    pub project_name: &'a str,
    pub firing: bool,
    pub metric: String,
    pub value: f64,
    pub threshold: f64,
    pub window_minutes: i32,
    pub environment: &'a str,
    pub query: &'a str,
    pub link: String,
    pub at_ms: i64,
}

pub fn rule_message(ev: &RuleEvent<'_>) -> RuleMessage {
    let state = if ev.firing { "Firing" } else { "Resolved" };
    let subject = format!(
        "[Trailscope] {state}: {} ({})",
        ev.rule_name, ev.project_name
    )
    .replace(['\r', '\n'], " ");
    let detail = format!(
        "{} is {} (threshold {}) over the last {} min{}{}.",
        ev.metric,
        fmt_value(ev.value),
        fmt_value(ev.threshold),
        ev.window_minutes,
        if ev.environment.is_empty() {
            String::new()
        } else {
            format!(" in {}", ev.environment)
        },
        if ev.query.is_empty() {
            String::new()
        } else {
            format!(", filter: {}", ev.query)
        }
    );
    let text = format!("{state}: {}\n{detail}\n{}", ev.rule_name, ev.link);
    let telegram = format!(
        "<b>{}: {}</b>\n{}\n<a href=\"{}\">Open in Trailscope</a>",
        esc(state),
        esc(ev.rule_name),
        esc(&detail),
        esc(&ev.link)
    );
    let color = if ev.firing { "#c0392b" } else { "#2e7d32" };
    let email_html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;background:#f6f7f9;font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#11151b">
<div style="max-width:600px;margin:0 auto;padding:24px 16px"><div style="background:#fff;border:1px solid #e1e4ea;border-radius:12px;padding:24px">
<p style="margin:0 0 4px;font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:#5b6575">Trailscope · {project}</p>
<h1 style="margin:0 0 12px;font-size:20px;color:{color}">{state}: {name}</h1>
<p style="margin:0 0 16px;font-size:15px;line-height:1.5">{detail}</p>
<a href="{link}" style="display:inline-block;padding:12px 18px;background:#11151b;color:#fff;text-decoration:none;border-radius:8px;font-size:14px;font-weight:600">Open in Trailscope</a>
</div></div></body></html>"#,
        project = esc(ev.project_name),
        state = state,
        name = esc(ev.rule_name),
        detail = esc(&detail),
        link = esc(&ev.link),
    );
    let body = serde_json::json!({
        "type": "alert",
        "state": if ev.firing { "firing" } else { "resolved" },
        "rule": { "id": ev.rule_id, "name": ev.rule_name },
        "project": ev.project_name,
        "environment": ev.environment,
        "metric": ev.metric,
        "value": ev.value,
        "threshold": ev.threshold,
        "windowMinutes": ev.window_minutes,
        "query": ev.query,
        "url": ev.link,
        "at": ev.at_ms,
    });
    RuleMessage {
        subject,
        text,
        telegram,
        email_html,
        body,
    }
}

fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}
