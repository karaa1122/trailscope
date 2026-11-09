//! Error fingerprints: xxhash64 of service + error type + the top 3
//! normalized stack frames. Without a stack, the message with numbers, UUIDs
//! and hex IDs replaced by placeholders is used instead.
//!
//! "Top" means innermost: the frame that threw. Python tracebacks list the
//! innermost frame last, so they are reversed. Framework frames
//! (node_modules, site-packages, runtime internals) are skipped when at least
//! one application frame is present, so the same bug hit through different
//! middleware stacks still groups together.

use std::sync::LazyLock;

use regex::Regex;
use xxhash_rust::xxh64::Xxh64;

static UUID: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b"));
static HEX_ID: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:0x)?[0-9a-f]{12,}\b"));
static NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"\d+(?:\.\d+)?"));
static LINE_COL: LazyLock<Regex> = LazyLock::new(|| re(r":\d+(?::\d+)?"));
static PY_LINE: LazyLock<Regex> = LazyLock::new(|| re(r", line \d+"));
static QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r#"'[^']*'|"[^"]*""#));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));

fn re(pattern: &str) -> Regex {
    // Patterns are constants covered by tests; an invalid one is a programming error.
    #[allow(clippy::expect_used)]
    Regex::new(pattern).expect("static regex")
}

const FRAMEWORK_MARKERS: &[&str] = &[
    "node_modules",
    "node:internal",
    "internal/",
    "site-packages",
    "dist-packages",
    "/lib/python",
    "<frozen ",
    "asyncio/",
    "java.base/",
    "jdk.internal.",
    "org.springframework.",
    "org.apache.catalina.",
    "org.apache.coyote.",
    "org.apache.tomcat.",
    "jakarta.servlet.",
    "/vendor/",
    "/rustc/",
];

pub fn is_framework_frame(frame: &str) -> bool {
    FRAMEWORK_MARKERS.iter().any(|m| frame.contains(m))
}

/// Computes the fingerprint, or 0 when there is nothing to group on.
pub fn fingerprint(service: &str, error_type: &str, message: &str, stack: &str) -> u64 {
    if error_type.is_empty() && message.is_empty() && stack.is_empty() {
        return 0;
    }
    let frames = top_frames(stack, 3);
    let mut h = Xxh64::new(0);
    h.update(service.as_bytes());
    h.update(b"\x1f");
    h.update(error_type.as_bytes());
    h.update(b"\x1f");
    if frames.is_empty() {
        h.update(normalize_message(message).as_bytes());
    } else {
        for f in &frames {
            h.update(f.as_bytes());
            h.update(b"\x1e");
        }
    }
    match h.digest() {
        0 => 1,
        d => d,
    }
}

/// The innermost `n` frames, normalized, preferring application frames.
pub fn top_frames(stack: &str, n: usize) -> Vec<String> {
    let mut frames = frames(stack);
    if frames.iter().any(|f| !is_framework_frame(f)) {
        frames.retain(|f| !is_framework_frame(f));
    }
    frames
        .into_iter()
        .take(n)
        .map(|f| normalize_frame(&f))
        .collect()
}

/// Raw frame lines, innermost first.
pub fn frames(stack: &str) -> Vec<String> {
    let python = stack.contains("Traceback (most recent call last)");
    let mut out: Vec<String> = stack
        .lines()
        .map(str::trim)
        .filter(|l| looks_like_frame(l))
        .map(str::to_string)
        .collect();
    if python {
        out.reverse();
    }
    out
}

fn looks_like_frame(line: &str) -> bool {
    line.starts_with("at ")                                  // JS, Java, C#
        || line.starts_with("File \"")                       // Python
        || (line.contains(".rs:") && line.contains("at "))   // Rust backtraces
        || line.contains(".go:")                             // Go
        || (line.contains('(') && line.contains(".java:"))
}

pub fn normalize_frame(frame: &str) -> String {
    let f = PY_LINE.replace_all(frame, "");
    let f = LINE_COL.replace_all(&f, "");
    let f = UUID.replace_all(&f, "<uuid>");
    let f = HEX_ID.replace_all(&f, "<id>");
    SPACES.replace_all(f.trim(), " ").into_owned()
}

pub fn normalize_message(message: &str) -> String {
    let m = UUID.replace_all(message, "<uuid>");
    let m = HEX_ID.replace_all(&m, "<id>");
    let m = QUOTED.replace_all(&m, "<str>");
    let m = NUMBER.replace_all(&m, "<n>");
    SPACES.replace_all(m.trim(), " ").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const JS_A: &str = "TimeoutError: upstream timed out after 3000ms (req 1f3a)\n    at callUpstream (/app/src/client.js:41:13)\n    at handler (/app/src/routes/users.js:12:5)\n    at Layer.handle (/app/node_modules/express/lib/router/layer.js:95:5)\n    at next (/app/node_modules/express/lib/router/route.js:137:13)\n    at main (/app/src/index.js:3:1)";
    const JS_B: &str = "TimeoutError: upstream timed out after 5000ms (req 99ff)\n    at callUpstream (/app/src/client.js:44:9)\n    at handler (/app/src/routes/users.js:15:5)\n    at Layer.handle (/app/node_modules/express/lib/router/layer.js:95:5)\n    at main (/app/src/index.js:3:1)";

    #[test]
    fn same_bug_different_lines_and_messages_groups_together() {
        let a = fingerprint(
            "api",
            "TimeoutError",
            "upstream timed out after 3000ms",
            JS_A,
        );
        let b = fingerprint(
            "api",
            "TimeoutError",
            "upstream timed out after 5000ms",
            JS_B,
        );
        assert_eq!(a, b);
        assert_ne!(a, 0);
    }

    #[test]
    fn service_and_type_split_groups() {
        let a = fingerprint("api", "TimeoutError", "", JS_A);
        assert_ne!(a, fingerprint("worker", "TimeoutError", "", JS_A));
        assert_ne!(a, fingerprint("api", "ValidationError", "", JS_A));
    }

    #[test]
    fn framework_frames_are_skipped() {
        let frames = top_frames(JS_A, 3);
        assert_eq!(frames.len(), 3);
        assert!(frames.iter().all(|f| !f.contains("node_modules")));
        assert_eq!(frames[0], "at callUpstream (/app/src/client.js)");
    }

    #[test]
    fn python_tracebacks_are_innermost_first() {
        let tb = "Traceback (most recent call last):\n  File \"/app/main.py\", line 10, in handler\n    do()\n  File \"/usr/lib/python3.13/site-packages/x.py\", line 5, in do\n    inner()\n  File \"/app/svc.py\", line 99, in inner\n    raise ValueError('bad id 42')\nValueError: bad id 42";
        let frames = top_frames(tb, 3);
        assert_eq!(frames[0], "File \"/app/svc.py\", in inner");
        assert_eq!(frames[1], "File \"/app/main.py\", in handler");
    }

    #[test]
    fn message_only_normalizes_ids_and_numbers() {
        let a = fingerprint(
            "api",
            "NotFound",
            "user 123 not found in 4f1c8e5a-2b7d-4c1e-9a3f-0e5d6c7b8a90",
            "",
        );
        let b = fingerprint(
            "api",
            "NotFound",
            "user 987 not found in 00000000-1111-2222-3333-444444444444",
            "",
        );
        assert_eq!(a, b);
        assert_eq!(
            normalize_message("order 'abc' failed after 3.5s, id deadbeefcafe1234"),
            "order <str> failed after <n>s, id <id>"
        );
    }

    #[test]
    fn nothing_to_group_on_is_zero() {
        assert_eq!(fingerprint("api", "", "", ""), 0);
    }
}
