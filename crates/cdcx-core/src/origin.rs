//! Origin-tagging for `client_oid` so orders placed via cdcx tooling are identifiable
//! in exchange-side databases. The exchange persists `client_oid` / `cl_order_id`
//! across its Postgres/ClickHouse/Cassandra stores, so a consistent 4-char prefix
//! lets operators bucket volume by channel with a simple `LEFT(cl_order_id, 3)`.
//!
//! # Prefix scheme
//!
//! | Channel | Prefix |
//! |---------|--------|
//! | CLI (`cdcx trade order ...`)       | `cx1-` |
//! | MCP server (AI agent tools)         | `cx2-` |
//! | TUI dashboard workflows             | `cx3-` |
//!
//! # Length constraint
//!
//! The exchange enforces a 36-char limit on `client_oid`. The prefix is always 4
//! chars, so the user's own ID (or auto-generated suffix) gets 32 chars of room.
//! User-supplied IDs are truncated to fit; callers can inspect the returned struct
//! to warn when truncation occurred.
//!
//! # Opt-out
//!
//! Setting `CDCX_NO_ORIGIN_TAG=1` disables tagging entirely. Intentionally undocumented
//! for external users — it exists for power users with pre-existing ID schemes that
//! must survive the round trip verbatim.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic counter so two calls within the same millisecond still produce distinct
/// suffixes. Stack addresses are not reliable entropy — the compiler may reuse them
/// across successive calls in a tight loop.
static SUFFIX_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Maximum accepted length of `client_oid` on the Crypto.com Exchange.
pub const MAX_CLIENT_OID_LEN: usize = 36;

/// Which surface of the cdcx tooling submitted this order. Determines the 3-char
/// origin code in the prefix (`cx1`/`cx2`/`cx3`).
///
/// Adding a new channel will produce compile errors at every call site, forcing
/// them to declare their origin — no silent "forgot to tag that one" bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginChannel {
    Cli,
    Mcp,
    Tui,
}

impl OriginChannel {
    /// The 4-char prefix written to `client_oid`, including the trailing hyphen.
    pub fn prefix(self) -> &'static str {
        match self {
            OriginChannel::Cli => "cx1-",
            OriginChannel::Mcp => "cx2-",
            OriginChannel::Tui => "cx3-",
        }
    }
}

/// Result of tagging an existing-or-absent `client_oid`. `truncated` tells callers
/// whether the user-supplied tail had to be shortened to fit the 36-char limit —
/// useful for stderr warnings in the CLI or toast notifications in the TUI.
#[derive(Debug, Clone)]
pub struct TaggedClientOid {
    pub value: String,
    pub truncated: bool,
}

/// Returns `true` when the caller has opted out of tagging via `CDCX_NO_ORIGIN_TAG=1`.
/// Any truthy value (`1`, `true`, `yes`) disables tagging; empty/unset/`0`/`false` do not.
fn opt_out() -> bool {
    match std::env::var("CDCX_NO_ORIGIN_TAG") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// Generate a ULID-ish time-ordered suffix, of exactly `len` chars. Uses crockford
/// base32 alphabet for friendliness and avoids characters that look alike (O/0, I/1).
/// The leading chars encode milliseconds since epoch, so sorting by `client_oid`
/// approximates chronological order — keeps Cassandra partition scans index-friendly.
fn generate_suffix(len: usize) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let mut buf = String::with_capacity(len);
    // Encode 48 bits of timestamp (enough through year 10889) into the first 10 chars
    // when space permits; remainder filled with pseudo-random base32 chars.
    let ts_chars = len.min(10);
    let mut ts = millis as u64;
    let mut ts_encoded = Vec::with_capacity(ts_chars);
    for _ in 0..ts_chars {
        ts_encoded.push(ALPHABET[(ts & 0x1f) as usize]);
        ts >>= 5;
    }
    ts_encoded.reverse();
    for byte in ts_encoded {
        buf.push(byte as char);
    }

    if buf.len() < len {
        // Fill remainder with entropy from the monotonic counter + millis. Uniqueness
        // is what matters here, not unpredictability — an adversary cannot derive
        // anything damaging from the suffix, and same-millisecond calls are handled
        // by the counter regardless of how fast the machine is.
        let counter = SUFFIX_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut seed = (millis as u64) ^ counter.rotate_left(17) ^ 0x9E3779B97F4A7C15;
        while buf.len() < len {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            buf.push(ALPHABET[((seed >> 33) & 0x1f) as usize] as char);
        }
    }

    buf
}

/// Tag a `client_oid` with the origin prefix. The returned string is guaranteed to be
/// ≤ 36 chars. If `existing` is provided, it's truncated to fit after the prefix; if
/// absent, a ULID-ish time-ordered suffix is generated.
///
/// Returns the original `existing` verbatim (or a generated suffix without prefix)
/// when the `CDCX_NO_ORIGIN_TAG` opt-out is active.
pub fn tag_client_oid(existing: Option<&str>, channel: OriginChannel) -> TaggedClientOid {
    if opt_out() {
        return TaggedClientOid {
            value: existing
                .map(|s| s.chars().take(MAX_CLIENT_OID_LEN).collect())
                .unwrap_or_else(|| generate_suffix(MAX_CLIENT_OID_LEN)),
            truncated: existing.is_some_and(|s| s.len() > MAX_CLIENT_OID_LEN),
        };
    }
    let prefix = channel.prefix();
    let budget = MAX_CLIENT_OID_LEN - prefix.len();
    match existing {
        Some(user) => {
            // If the user already tagged the ID themselves (e.g. replayed through cdcx
            // a second time, or the TUI handing to the same helper twice), leave it
            // alone so we never double-prefix cx1-cx1-...
            if user.starts_with("cx1-") || user.starts_with("cx2-") || user.starts_with("cx3-") {
                let truncated = user.len() > MAX_CLIENT_OID_LEN;
                return TaggedClientOid {
                    value: user.chars().take(MAX_CLIENT_OID_LEN).collect(),
                    truncated,
                };
            }
            let truncated = user.len() > budget;
            let tail: String = user.chars().take(budget).collect();
            TaggedClientOid {
                value: format!("{}{}", prefix, tail),
                truncated,
            }
        }
        None => TaggedClientOid {
            value: format!("{}{}", prefix, generate_suffix(budget)),
            truncated: false,
        },
    }
}

/// Tag every leg in an OCO / OTOCO `order_list` array under a `params` object.
/// No-op if the object doesn't contain `order_list` or it isn't an array. Returns
/// the count of legs that had their `client_oid` truncated, so callers can warn once.
pub fn tag_order_list_legs(params: &mut serde_json::Value, channel: OriginChannel) -> usize {
    let mut truncations = 0;
    let Some(obj) = params.as_object_mut() else {
        return 0;
    };
    let Some(list) = obj.get_mut("order_list").and_then(|v| v.as_array_mut()) else {
        return 0;
    };
    for leg in list.iter_mut() {
        let Some(leg_obj) = leg.as_object_mut() else {
            continue;
        };
        let existing = leg_obj
            .get("client_oid")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let tagged = tag_client_oid(existing.as_deref(), channel);
        if tagged.truncated {
            truncations += 1;
        }
        leg_obj.insert("client_oid".into(), serde_json::Value::String(tagged.value));
    }
    truncations
}

/// Tag the top-level `client_oid` on a `create-order` / `create-order-list` / etc.
/// payload in place. Returns the tagged `client_oid` so callers can surface it (e.g.
/// print it to stderr after placement). A `None` return means the opt-out was active
/// and the field was left untouched when absent, or returned verbatim when present.
pub fn tag_params_in_place(
    params: &mut serde_json::Value,
    channel: OriginChannel,
) -> Option<TaggedClientOid> {
    let obj = params.as_object_mut()?;
    let existing = obj
        .get("client_oid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let tagged = tag_client_oid(existing.as_deref(), channel);
    obj.insert(
        "client_oid".into(),
        serde_json::Value::String(tagged.value.clone()),
    );
    Some(tagged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-var mutation across parallel tests. `#[test]`s run on multiple
    /// threads in one process; without this lock, `CDCX_NO_ORIGIN_TAG` flips under
    /// our feet and causes flaky assertions.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Execute `f` with `CDCX_NO_ORIGIN_TAG` temporarily unset, so tests are not
    /// influenced by the developer's local env.
    fn with_tagging_enabled<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("CDCX_NO_ORIGIN_TAG").ok();
        std::env::remove_var("CDCX_NO_ORIGIN_TAG");
        let out = f();
        if let Some(v) = prev {
            std::env::set_var("CDCX_NO_ORIGIN_TAG", v);
        }
        out
    }

    #[test]
    fn prefix_is_four_chars_with_hyphen() {
        assert_eq!(OriginChannel::Cli.prefix(), "cx1-");
        assert_eq!(OriginChannel::Mcp.prefix(), "cx2-");
        assert_eq!(OriginChannel::Tui.prefix(), "cx3-");
        for ch in [OriginChannel::Cli, OriginChannel::Mcp, OriginChannel::Tui] {
            assert_eq!(ch.prefix().len(), 4);
            assert!(ch.prefix().ends_with('-'));
        }
    }

    #[test]
    fn autogenerated_oid_respects_limit() {
        with_tagging_enabled(|| {
            let tagged = tag_client_oid(None, OriginChannel::Cli);
            assert!(tagged.value.starts_with("cx1-"));
            assert_eq!(tagged.value.len(), MAX_CLIENT_OID_LEN);
            assert!(!tagged.truncated);
        });
    }

    #[test]
    fn user_oid_gets_prefixed() {
        with_tagging_enabled(|| {
            let tagged = tag_client_oid(Some("my-ladder-3"), OriginChannel::Cli);
            assert_eq!(tagged.value, "cx1-my-ladder-3");
            assert!(!tagged.truncated);
        });
    }

    #[test]
    fn overlong_user_oid_is_truncated_prefix_preserved() {
        with_tagging_enabled(|| {
            let long = "a".repeat(100);
            let tagged = tag_client_oid(Some(&long), OriginChannel::Tui);
            assert!(tagged.value.starts_with("cx3-"));
            assert_eq!(tagged.value.len(), MAX_CLIENT_OID_LEN);
            assert!(tagged.truncated);
        });
    }

    #[test]
    fn already_tagged_oid_not_double_prefixed() {
        with_tagging_enabled(|| {
            let tagged = tag_client_oid(Some("cx1-pre-existing-id"), OriginChannel::Mcp);
            // Keep the original tag — do not wrap cx2-cx1-...
            assert_eq!(tagged.value, "cx1-pre-existing-id");
        });
    }

    #[test]
    fn opt_out_env_var_suppresses_prefix() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("CDCX_NO_ORIGIN_TAG", "1");
        let tagged = tag_client_oid(Some("plain-id"), OriginChannel::Cli);
        assert_eq!(tagged.value, "plain-id");
        std::env::remove_var("CDCX_NO_ORIGIN_TAG");
    }

    #[test]
    fn tag_params_in_place_adds_client_oid_when_absent() {
        with_tagging_enabled(|| {
            let mut params = serde_json::json!({"side": "BUY"});
            let tagged = tag_params_in_place(&mut params, OriginChannel::Mcp).unwrap();
            assert!(tagged.value.starts_with("cx2-"));
            assert_eq!(
                params["client_oid"],
                serde_json::Value::String(tagged.value)
            );
        });
    }

    #[test]
    fn tag_order_list_legs_tags_each_leg() {
        with_tagging_enabled(|| {
            let mut params = serde_json::json!({
                "contingency_type": "OCO",
                "order_list": [
                    {"side": "SELL", "client_oid": "leg-a"},
                    {"side": "SELL"},
                ]
            });
            let truncations = tag_order_list_legs(&mut params, OriginChannel::Tui);
            assert_eq!(truncations, 0);
            let list = params["order_list"].as_array().unwrap();
            assert_eq!(
                list[0]["client_oid"],
                serde_json::Value::String("cx3-leg-a".into())
            );
            let generated = list[1]["client_oid"].as_str().unwrap();
            assert!(generated.starts_with("cx3-"));
            assert_eq!(generated.len(), MAX_CLIENT_OID_LEN);
        });
    }

    #[test]
    fn generated_suffixes_are_unique_across_rapid_calls() {
        with_tagging_enabled(|| {
            let a = tag_client_oid(None, OriginChannel::Cli).value;
            let b = tag_client_oid(None, OriginChannel::Cli).value;
            let c = tag_client_oid(None, OriginChannel::Cli).value;
            assert_ne!(a, b);
            assert_ne!(b, c);
            assert_ne!(a, c);
        });
    }
}
