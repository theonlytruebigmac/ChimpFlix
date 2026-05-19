//! Webhook dispatcher: subscribes to `Event::Webhook`, fans out to every
//! enabled subscriber whose `event_mask` matches, signs the payload with
//! HMAC-SHA256 when a secret is set, and records the attempt for the
//! admin UI.
//!
//! Retries: 3 attempts total with exponential backoff (60s, 300s, 1800s).
//! A separate background task re-attempts pending deliveries whose
//! `next_retry_at` has passed; failures past attempt 3 are left in the
//! table for diagnostics.

use std::time::Duration;

use chimpflix_common::now_ms;
use chimpflix_library::{Webhook, queries};
use hmac::{Hmac, Mac};
use rand_core::RngCore;
use sha2::Sha256;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

use crate::events::{Event, WebhookEvent};
use crate::state::AppState;

const MAX_ATTEMPTS: i64 = 3;
const BACKOFF_MS: [i64; 3] = [60_000, 300_000, 1_800_000];

/// Apply a ±10% multiplicative jitter to a retry backoff. Without
/// jitter, every webhook that fails its first attempt at the same
/// time retries at the same instant — a thundering herd against the
/// receiver. The OS RNG is plenty for non-cryptographic spread.
fn jittered_backoff_ms(base_ms: i64) -> i64 {
    // basis-point offset in [-1000, 1000) → ±10% of base.
    let r = rand_core::OsRng.next_u32() % 2000;
    let bps_offset = r as i64 - 1000;
    let scaled = base_ms.saturating_mul(10_000_i64.saturating_add(bps_offset)) / 10_000;
    scaled.max(1)
}

/// Walk a JSON value and replace any field whose key looks
/// secret-shaped with the literal string `"***"`. Defense in depth
/// for the stored-payload column in `webhook_deliveries` — the events
/// we emit today don't include credentials, but a future event might
/// and we'd rather the DB column be safe-by-default than hope every
/// future contributor remembers. Matches case-insensitively against
/// common substrings used across our codebase + Trakt + TMDB + OAuth.
fn mask_sensitive_json_in_place(v: &mut serde_json::Value) {
    const SENSITIVE_KEY_SUBSTRINGS: &[&str] = &[
        "password",
        "secret",
        "token",
        "api_key",
        "apikey",
        "authorization",
        "refresh_token",
        "access_token",
        "session_id",
        "csrf",
    ];
    match v {
        serde_json::Value::Object(map) => {
            for (k, val) in map.iter_mut() {
                let lk = k.to_ascii_lowercase();
                if SENSITIVE_KEY_SUBSTRINGS.iter().any(|p| lk.contains(p)) {
                    *val = serde_json::Value::String("***".to_string());
                } else {
                    mask_sensitive_json_in_place(val);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                mask_sensitive_json_in_place(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod mask_tests {
    use super::mask_sensitive_json_in_place;
    use serde_json::json;

    #[test]
    fn masks_top_level_token() {
        let mut v = json!({"id": 7, "token": "abc"});
        mask_sensitive_json_in_place(&mut v);
        assert_eq!(v["id"], 7);
        assert_eq!(v["token"], "***");
    }

    #[test]
    fn masks_nested_secrets() {
        let mut v = json!({
            "data": { "user": { "api_key": "k", "name": "alice" } }
        });
        mask_sensitive_json_in_place(&mut v);
        assert_eq!(v["data"]["user"]["api_key"], "***");
        assert_eq!(v["data"]["user"]["name"], "alice");
    }

    #[test]
    fn masks_inside_arrays() {
        let mut v = json!({ "creds": [ {"refresh_token": "x"}, {"keep": 1} ] });
        mask_sensitive_json_in_place(&mut v);
        assert_eq!(v["creds"][0]["refresh_token"], "***");
        assert_eq!(v["creds"][1]["keep"], 1);
    }

    #[test]
    fn case_insensitive_match() {
        let mut v = json!({ "API_KEY": "x", "AccessToken": "y" });
        mask_sensitive_json_in_place(&mut v);
        assert_eq!(v["API_KEY"], "***");
        assert_eq!(v["AccessToken"], "***");
    }
}

pub fn spawn(state: AppState) {
    // Bus subscriber: fan out to matching webhooks.
    let dispatch_state = state.clone();
    tokio::spawn(async move {
        let mut rx = dispatch_state.hub.subscribe();
        loop {
            match rx.recv().await {
                Ok(Event::Webhook(evt)) => fan_out(&dispatch_state, evt).await,
                Ok(Event::Scan(_)) | Ok(Event::Sessions(_)) => {}
                Err(RecvError::Lagged(n)) => {
                    warn!(skipped = n, "webhook subscriber lagged; events lost");
                }
                Err(RecvError::Closed) => break,
            }
        }
    });

    // Retry loop: every 30s, re-attempt deliveries whose next_retry_at has
    // passed and attempts < MAX_ATTEMPTS.
    let retry_state = state;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.tick().await;
        loop {
            tick.tick().await;
            if let Err(e) = retry_pending(&retry_state).await {
                warn!(error = %format!("{e:#}"), "webhook retry tick failed");
            }
        }
    });
}

async fn fan_out(state: &AppState, evt: WebhookEvent) {
    let hooks = match queries::list_webhooks(&state.pool, &state.vault).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %format!("{e:#}"), "failed to list webhooks");
            return;
        }
    };
    for hook in hooks {
        if !hook.enabled {
            continue;
        }
        let mask: Vec<String> = serde_json::from_str(&hook.event_mask).unwrap_or_default();
        if !mask.iter().any(|n| n == &evt.name) {
            continue;
        }
        deliver_async(state.clone(), hook, evt.clone()).await;
    }
}

async fn deliver_async(state: AppState, hook: Webhook, evt: WebhookEvent) {
    let mut payload = serde_json::json!({ "event": evt.name, "data": evt.payload });
    let payload_str = payload.to_string();
    // Mask any sensitive fields in the stored copy before it lands in
    // the admin-visible `webhook_deliveries` table. We don't mask the
    // bytes we actually POST to the receiver — those are payload_str
    // above — so signatures remain verifiable. The masked variant
    // exists solely for the operator-facing log so a misbehaving
    // event payload (or a future event we add) can't leak a secret
    // into a DB column.
    mask_sensitive_json_in_place(&mut payload);
    let stored_payload = payload.to_string();
    let delivery_id = match queries::create_webhook_delivery(
        &state.pool,
        hook.id,
        &evt.name,
        &stored_payload,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!(error = %format!("{e:#}"), webhook_id = hook.id, "could not record delivery");
            return;
        }
    };
    tokio::spawn(async move {
        deliver_with_retries(state, hook, delivery_id, payload_str, 0).await;
    });
}

/// Iterative retry driver. Single async fn (no recursion through
/// `tokio::spawn`) so the future is unambiguously `Send`.
async fn deliver_with_retries(
    state: AppState,
    hook: Webhook,
    delivery_id: i64,
    payload: String,
    start_attempt_idx: usize,
) {
    let mut attempt_idx = start_attempt_idx;
    loop {
        let outcome = attempt_once(&state, &hook, delivery_id, &payload).await;
        match outcome {
            AttemptOutcome::Success => {
                debug!(webhook_id = hook.id, delivery_id, "webhook delivered");
                return;
            }
            AttemptOutcome::Retry { code, body, error } => {
                let next_attempt = attempt_idx + 1;
                if (next_attempt as i64) >= MAX_ATTEMPTS {
                    let _ = queries::record_webhook_attempt(
                        &state.pool,
                        delivery_id,
                        code,
                        body.as_deref(),
                        error.as_deref(),
                        false,
                        None,
                    )
                    .await;
                    warn!(
                        webhook_id = hook.id,
                        delivery_id, "webhook delivery exhausted retries"
                    );
                    return;
                }
                let base_backoff =
                    BACKOFF_MS.get(attempt_idx).copied().unwrap_or(1_800_000);
                let backoff = jittered_backoff_ms(base_backoff);
                let next_at = now_ms() + backoff;
                let _ = queries::record_webhook_attempt(
                    &state.pool,
                    delivery_id,
                    code,
                    body.as_deref(),
                    error.as_deref(),
                    false,
                    Some(next_at),
                )
                .await;
                tokio::time::sleep(Duration::from_millis(backoff as u64)).await;
                attempt_idx = next_attempt;
            }
        }
    }
}

enum AttemptOutcome {
    Success,
    Retry {
        code: Option<i64>,
        body: Option<String>,
        error: Option<String>,
    },
}

async fn attempt_once(
    state: &AppState,
    hook: &Webhook,
    delivery_id: i64,
    payload: &str,
) -> AttemptOutcome {
    // SSRF guard: resolve the URL's hostname and reject before we
    // open the connection if it points at loopback / RFC1918 / link-local
    // / cloud-metadata. Without this, an owner-compromised webhook URL
    // could exfiltrate IAM credentials via 169.254.169.254 and the
    // response body would be captured straight into webhook_deliveries
    // for the admin UI to render.
    if let Err(reason) = crate::ssrf::ensure_safe_outbound_url(&hook.url).await {
        let _ = queries::record_webhook_attempt(
            &state.pool,
            delivery_id,
            None,
            None,
            Some(&format!("ssrf-blocked: {reason}")),
            false,
            None,
        )
        .await;
        return AttemptOutcome::Retry {
            code: None,
            body: None,
            error: Some(format!("ssrf-blocked: {reason}")),
        };
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = queries::record_webhook_attempt(
                &state.pool,
                delivery_id,
                None,
                None,
                Some(&format!("client build: {e}")),
                false,
                None,
            )
            .await;
            return AttemptOutcome::Retry {
                code: None,
                body: None,
                error: Some(format!("client build: {e}")),
            };
        }
    };

    let signature = hook.secret.as_deref().map(|secret| sign(payload, secret));
    let mut req = client
        .post(&hook.url)
        .header("Content-Type", "application/json")
        .header("X-ChimpFlix-Delivery", delivery_id.to_string())
        .body(payload.to_string());
    if let Some(sig) = signature {
        req = req.header("X-ChimpFlix-Signature", format!("sha256={sig}"));
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let code = status.as_u16() as i64;
            // Cap the response body we persist. The receiver might
            // echo our payload back (especially on validation errors)
            // and we don't want to forever-store secrets we just
            // masked above. 1024 chars is plenty to diagnose a
            // misbehaving endpoint without copy-pasting half its
            // request log into our DB.
            let body = resp
                .text()
                .await
                .ok()
                .map(|s| s.chars().take(1024).collect::<String>());
            if status.is_success() {
                let _ = queries::record_webhook_attempt(
                    &state.pool,
                    delivery_id,
                    Some(code),
                    body.as_deref(),
                    None,
                    true,
                    None,
                )
                .await;
                AttemptOutcome::Success
            } else {
                AttemptOutcome::Retry {
                    code: Some(code),
                    body,
                    error: None,
                }
            }
        }
        Err(e) => AttemptOutcome::Retry {
            code: None,
            body: None,
            error: Some(format!("{e}")),
        },
    }
}

async fn retry_pending(state: &AppState) -> anyhow::Result<()> {
    // Belt-and-braces against missed in-process retries: scan for any
    // deliveries past their next_retry_at and re-fire.
    let now = now_ms();
    let rows = sqlx::query(
        "SELECT d.id, d.webhook_id, d.payload_json, d.attempts
         FROM webhook_deliveries d
         WHERE d.delivered_at IS NULL
           AND d.next_retry_at IS NOT NULL
           AND d.next_retry_at <= ?
           AND d.attempts < ?",
    )
    .bind(now)
    .bind(MAX_ATTEMPTS)
    .fetch_all(&state.pool)
    .await?;
    for row in &rows {
        use sqlx::Row;
        let delivery_id: i64 = row.try_get("id").unwrap_or(0);
        let webhook_id: i64 = row.try_get("webhook_id").unwrap_or(0);
        let payload: String = row.try_get("payload_json").unwrap_or_default();
        let attempts: i64 = row.try_get("attempts").unwrap_or(0);
        let Some(hook) = queries::get_webhook(&state.pool, &state.vault, webhook_id).await? else {
            continue;
        };
        let st = state.clone();
        tokio::spawn(async move {
            deliver_with_retries(st, hook, delivery_id, payload, attempts as usize).await;
        });
    }
    Ok(())
}

fn sign(payload: &str, secret: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("hmac accepts any key length");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Public list of event names a webhook can subscribe to. Keep in sync
/// with every `hub.publish(Event::Webhook(...))` site in the codebase.
pub fn event_catalog() -> Vec<&'static str> {
    vec![
        "webhook.test",
        "scan.completed",
        "item.added",
        "item.updated",
        "play.started",
        "play.stopped",
        "play.scrobbled",
        "marker.detected",
        "user.created",
    ]
}
