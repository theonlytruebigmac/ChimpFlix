//! /admin/webhooks* — CRUD, test-fire, and delivery history.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
use chimpflix_library::{
    NewAuditEntry, NewWebhook, Webhook, WebhookDelivery, WebhookUpdate, queries,
};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::events::{Event, WebhookEvent};
use crate::state::AppState;
use crate::webhooks;

#[derive(Debug, Serialize)]
pub struct WebhooksListResponse {
    pub webhooks: Vec<Webhook>,
    pub events: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub webhook: Webhook,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<WebhooksListResponse>, ApiError> {
    let webhooks = queries::list_webhooks(&state.pool, &state.vault)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(WebhooksListResponse {
        webhooks,
        events: webhooks::event_catalog(),
    }))
}

pub async fn create(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewWebhook>,
) -> Result<(StatusCode, Json<WebhookResponse>), ApiError> {
    validate_new(&input).await?;
    let webhook = queries::create_webhook(&state.pool, &state.vault, input.clone())
        .await
        .map_err(ApiError::Internal)?;
    audit(
        &state,
        actor.id,
        &headers,
        "webhook.create",
        webhook.id,
        &input,
    )
    .await;
    Ok((StatusCode::CREATED, Json(WebhookResponse { webhook })))
}

pub async fn update(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<WebhookUpdate>,
) -> Result<Json<WebhookResponse>, ApiError> {
    if let Some(ref url) = input.url {
        validate_url_async(url).await?;
    }
    if let Some(ref mask) = input.event_mask {
        validate_event_mask(mask)?;
    }
    let webhook = queries::update_webhook(&state.pool, &state.vault, id, input.clone())
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    audit(&state, actor.id, &headers, "webhook.update", id, &input).await;
    Ok(Json(WebhookResponse { webhook }))
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let removed = queries::delete_webhook(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    audit(&state, actor.id, &headers, "webhook.delete", id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn test_fire(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let hook = queries::get_webhook(&state.pool, &state.vault, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    // Direct-fire: bypass the event_mask filter so the test always reaches
    // the chosen webhook regardless of subscription state.
    let payload = serde_json::json!({
        "event": "webhook.test",
        "data": {
            "message": "Test fire from ChimpFlix admin",
            "now_ms": chimpflix_common::now_ms(),
            "fired_by_user_id": actor.id,
        }
    });
    let payload_str = payload.to_string();
    let delivery_id =
        queries::create_webhook_delivery(&state.pool, hook.id, "webhook.test", &payload_str)
            .await
            .map_err(ApiError::Internal)?;
    let st = state.clone();
    tokio::spawn(async move {
        // Reuse the dispatcher's attempt path indirectly by publishing a
        // matching mask-targeted event would route through filters; here
        // we want a direct deliver. Issue a hand-rolled request matching
        // the production format.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let Ok(client) = client else { return };
        let sig = hook
            .secret
            .as_deref()
            .map(|secret| sign_hex(&payload_str, secret));
        let mut req = client
            .post(&hook.url)
            .header("Content-Type", "application/json")
            .header("X-ChimpFlix-Delivery", delivery_id.to_string())
            .header("X-ChimpFlix-Test", "1")
            .body(payload_str.clone());
        if let Some(sig) = sig {
            req = req.header("X-ChimpFlix-Signature", format!("sha256={sig}"));
        }
        match req.send().await {
            Ok(resp) => {
                let code = resp.status().as_u16() as i64;
                let body = resp.text().await.ok();
                let _ = queries::record_webhook_attempt(
                    &st.pool,
                    delivery_id,
                    Some(code),
                    body.as_deref(),
                    None,
                    (200..300).contains(&code),
                    None,
                )
                .await;
            }
            Err(e) => {
                let _ = queries::record_webhook_attempt(
                    &st.pool,
                    delivery_id,
                    None,
                    None,
                    Some(&format!("{e}")),
                    false,
                    None,
                )
                .await;
            }
        }
    });
    audit(&state, actor.id, &headers, "webhook.test", id, &()).await;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize)]
pub struct DeliveriesParams {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DeliveriesResponse {
    pub deliveries: Vec<WebhookDelivery>,
}

pub async fn list_deliveries(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
    Query(params): Query<DeliveriesParams>,
) -> Result<Json<DeliveriesResponse>, ApiError> {
    let deliveries = queries::list_webhook_deliveries(&state.pool, id, params.limit.unwrap_or(50))
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(DeliveriesResponse { deliveries }))
}

async fn validate_new(input: &NewWebhook) -> Result<(), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::validation("name is required"));
    }
    validate_url_async(&input.url).await?;
    validate_event_mask(&input.event_mask)?;
    Ok(())
}

async fn validate_url_async(url: &str) -> Result<(), ApiError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ApiError::validation(
            "url must start with http:// or https://",
        ));
    }
    // SSRF guard at write time. The dispatcher re-validates at request
    // time as a TOCTOU defence; rejecting at the admin PATCH gives the
    // operator immediate feedback instead of a silent "no deliveries
    // ever happen" mystery.
    if let Err(reason) = crate::ssrf::ensure_safe_outbound_url(url).await {
        return Err(ApiError::validation(format!("url rejected: {reason}")));
    }
    Ok(())
}

fn validate_event_mask(mask: &[String]) -> Result<(), ApiError> {
    if mask.is_empty() {
        return Err(ApiError::validation(
            "event_mask must contain at least one event name",
        ));
    }
    let catalog = webhooks::event_catalog();
    for name in mask {
        if !catalog.contains(&name.as_str()) {
            return Err(ApiError::validation(format!(
                "unknown event `{name}` — valid events: {}",
                catalog.join(", ")
            )));
        }
    }
    Ok(())
}

fn sign_hex(payload: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

async fn audit<T: Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    target_id: i64,
    payload: &T,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor_id),
            action: action.to_string(),
            target_kind: Some("webhook".into()),
            target_id: Some(target_id.to_string()),
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}

// Suppress unused warnings — `WebhookEvent` and `Event` are imported for
// dispatcher symmetry; future code that publishes through this module
// will use them.
#[allow(dead_code)]
fn _unused(_: WebhookEvent, _: Event) {}
