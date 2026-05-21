//! `/admin/secrets/*` — credential vault management.
//!
//! Exposes the named-secret surface of [`chimpflix_common::Vault`] to the
//! owner UI. The plaintext value of any secret is never returned in a
//! response; the listing returns metadata plus a masked `last4`.
//!
//! Slots are a closed set defined by [`KNOWN_SLOTS`]. Adding a new
//! integration means adding a new entry here so the UI can render a card
//! for it even when it isn't set yet.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, SecretMetadata, queries};
use chimpflix_metadata::{
    AniListClient, OpenSubtitlesClient, OpenSubtitlesCreds, TmdbClient, TraktClient, TraktCreds,
    TvdbClient,
};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

/// Closed registry of credential slots. `managed = true` means the slot is
/// written by the server itself (currently the session HMAC) and the API
/// refuses owner-driven set/delete on it so the operator can't sign every
/// user out by accident.
struct SlotSpec {
    name: &'static str,
    display_name: &'static str,
    description: &'static str,
    managed: bool,
}

const KNOWN_SLOTS: &[SlotSpec] = &[
    SlotSpec {
        name: "tmdb",
        display_name: "TMDB",
        description: "The Movie Database v4 read token. Primary source of \
                      movie and TV metadata, posters, credits, and reviews.",
        managed: false,
    },
    SlotSpec {
        name: "tvdb",
        display_name: "TheTVDB",
        description: "TheTVDB v4 API key. Backfill metadata source for TV \
                      shows; complements TMDB on long-running series.",
        managed: false,
    },
    SlotSpec {
        name: "anilist",
        display_name: "AniList",
        description: "Leave blank — metadata enrichment works without it \
                      (30 requests/minute, plenty for library scans). \
                      If you do set it, this field expects an AniList \
                      OAuth Access Token (the long string you get back \
                      AFTER walking through the authorize redirect at \
                      anilist.co/api/v2/oauth/authorize?client_id=…&response_type=token). \
                      It is NOT your Client Secret from anilist.co/settings/developer — \
                      pasting the Secret here will trigger 400 Invalid token \
                      errors on every request.",
        managed: false,
    },
    SlotSpec {
        name: "opensubtitles",
        display_name: "OpenSubtitles",
        description: "JSON triple: {\"api_key\":\"…\",\"username\":\"…\",\"password\":\"…\"}. \
                      The api_key comes from registering an app at opensubtitles.com; \
                      username/password are needed for the /download endpoint.",
        managed: false,
    },
    SlotSpec {
        name: "trakt",
        display_name: "Trakt",
        description: "JSON pair: {\"client_id\":\"…\",\"client_secret\":\"…\"} from a \
                      Trakt OAuth app you've registered at trakt.tv/oauth/applications. \
                      Use redirect URI `urn:ietf:wg:oauth:2.0:oob` so the device-code \
                      link flow works. Per-user access tokens are minted by users via \
                      the Link Trakt button under their settings.",
        managed: false,
    },
    SlotSpec {
        name: "omdb",
        display_name: "OMDb",
        description: "OMDb API key — fuels the external ratings handler \
                      (IMDb, Rotten Tomatoes, Metacritic, MPAA). Get a \
                      free key at omdbapi.com (1,000 requests/day) or \
                      a paid Patreon key for higher quotas.",
        managed: false,
    },
    SlotSpec {
        name: "session_hmac",
        display_name: "Session HMAC",
        description: "Signs your session cookies. System-managed — \
                      rotating it signs every user out, so it cannot be \
                      cleared from here.",
        managed: true,
    },
];

fn lookup_slot(name: &str) -> Option<&'static SlotSpec> {
    KNOWN_SLOTS.iter().find(|s| s.name == name)
}

#[derive(Debug, Serialize)]
pub struct SlotView {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub managed: bool,
    pub stored: Option<SecretMetadata>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    /// `true` when CHIMPFLIX_SECRET_KEY is set and stored values are
    /// encrypted at rest. The UI uses this to drive the "your secrets are
    /// in plaintext" red banner.
    pub encrypted_at_rest: bool,
    pub slots: Vec<SlotView>,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<ListResponse>, ApiError> {
    let stored = queries::vault_list_metadata(&state.pool, &state.vault)
        .await
        .map_err(ApiError::Internal)?;
    // Index by name for O(1) lookup while preserving slot ordering.
    let mut stored_by_name: std::collections::HashMap<String, SecretMetadata> =
        stored.into_iter().map(|s| (s.name.clone(), s)).collect();

    let slots = KNOWN_SLOTS
        .iter()
        .map(|spec| SlotView {
            name: spec.name.into(),
            display_name: spec.display_name.into(),
            description: spec.description.into(),
            managed: spec.managed,
            stored: stored_by_name.remove(spec.name),
        })
        .collect();

    Ok(Json(ListResponse {
        encrypted_at_rest: state.vault.is_encrypted(),
        slots,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SetInput {
    pub value: String,
}

pub async fn put(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(input): Json<SetInput>,
) -> Result<Json<SlotView>, ApiError> {
    let spec = lookup_slot(&name).ok_or_else(|| {
        ApiError::validation(format!("unknown secret slot: {name}"))
    })?;
    if spec.managed {
        return Err(ApiError::validation(format!(
            "secret slot '{}' is system-managed and cannot be set from here",
            spec.name
        )));
    }
    let trimmed = input.value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::validation("value must not be empty — use DELETE to clear"));
    }

    queries::vault_set(&state.pool, &state.vault, spec.name, trimmed, Some(actor.id))
        .await
        .map_err(ApiError::Internal)?;
    refresh_runtime_client(&state, spec.name, Some(trimmed)).await?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "secrets.set".into(),
            target_kind: Some("secret".into()),
            target_id: Some(spec.name.into()),
            // Deliberately no payload — never log the credential value.
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;

    slot_view(&state, spec).await
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SlotView>, ApiError> {
    let spec = lookup_slot(&name).ok_or_else(|| {
        ApiError::validation(format!("unknown secret slot: {name}"))
    })?;
    if spec.managed {
        return Err(ApiError::validation(format!(
            "secret slot '{}' is system-managed and cannot be cleared from here",
            spec.name
        )));
    }

    queries::vault_delete(&state.pool, spec.name)
        .await
        .map_err(ApiError::Internal)?;
    refresh_runtime_client(&state, spec.name, None).await?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "secrets.delete".into(),
            target_kind: Some("secret".into()),
            target_id: Some(spec.name.into()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;

    slot_view(&state, spec).await
}

#[derive(Debug, Default, Deserialize)]
pub struct TestInput {
    /// When provided, test the candidate value before saving. When absent,
    /// test the value currently in the vault — useful for an "is it still
    /// working?" button on already-configured slots.
    pub value: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestResponse {
    pub ok: bool,
    pub detail: String,
}

pub async fn test(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(name): Path<String>,
    body: Option<Json<TestInput>>,
) -> Result<Json<TestResponse>, ApiError> {
    let spec = lookup_slot(&name).ok_or_else(|| {
        ApiError::validation(format!("unknown secret slot: {name}"))
    })?;

    let candidate = body
        .and_then(|Json(input)| input.value)
        .and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() { None } else { Some(t) }
        });

    let value = match candidate {
        Some(v) => v,
        None => match queries::vault_get(&state.pool, &state.vault, spec.name)
            .await
            .map_err(ApiError::Internal)?
        {
            Some(v) => v,
            None => {
                return Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("'{}' is not set", spec.name),
                }));
            }
        },
    };

    match spec.name {
        "tmdb" => match TmdbClient::new(&value) {
            Ok(client) => match client.validate().await {
                Ok(base) => Ok(Json(TestResponse {
                    ok: true,
                    detail: format!("TMDB v4 accepted the key (images served from {base})"),
                })),
                Err(e) => Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("TMDB rejected the key: {e:#}"),
                })),
            },
            Err(e) => Ok(Json(TestResponse {
                ok: false,
                detail: format!("token is not valid ASCII: {e:#}"),
            })),
        },
        "tvdb" => match TvdbClient::new(&value, None) {
            Ok(client) => match client.validate().await {
                Ok(()) => Ok(Json(TestResponse {
                    ok: true,
                    detail: "TheTVDB accepted the key".into(),
                })),
                Err(e) => Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("TheTVDB rejected the key: {e:#}"),
                })),
            },
            Err(e) => Ok(Json(TestResponse {
                ok: false,
                detail: format!("TVDB key not usable: {e:#}"),
            })),
        },
        "anilist" => match AniListClient::with_token(&value) {
            Ok(client) => match client.validate().await {
                Ok(()) => Ok(Json(TestResponse {
                    ok: true,
                    detail: "AniList accepted the token".into(),
                })),
                Err(e) => Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("AniList rejected the token: {e:#}"),
                })),
            },
            Err(e) => Ok(Json(TestResponse {
                ok: false,
                detail: format!("AniList client init failed: {e:#}"),
            })),
        },
        "opensubtitles" => match OpenSubtitlesCreds::parse(&value)
            .and_then(OpenSubtitlesClient::new)
        {
            Ok(client) => match client.validate().await {
                Ok(()) => Ok(Json(TestResponse {
                    ok: true,
                    detail: "OpenSubtitles accepted the credentials".into(),
                })),
                Err(e) => Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("OpenSubtitles rejected: {e:#}"),
                })),
            },
            Err(e) => Ok(Json(TestResponse {
                ok: false,
                detail: format!("OpenSubtitles credentials are not usable: {e:#}"),
            })),
        },
        "trakt" => match TraktCreds::parse(&value).and_then(TraktClient::from_creds) {
            Ok(client) => match client.validate().await {
                Ok(()) => Ok(Json(TestResponse {
                    ok: true,
                    detail: "Trakt accepted the OAuth app credentials".into(),
                })),
                Err(e) => Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("Trakt rejected: {e:#}"),
                })),
            },
            Err(e) => Ok(Json(TestResponse {
                ok: false,
                detail: format!("Trakt credentials not usable: {e:#}"),
            })),
        },
        "omdb" => {
            // We don't ship a dedicated /validate endpoint for OMDb —
            // a basic "construct the client and try a known lookup"
            // proves the key works. Use a stable IMDb id (Citizen
            // Kane) since OMDb's archive is unlikely to lose it.
            match chimpflix_metadata::OmdbClient::new(value.clone()) {
                Ok(client) => match client.fetch_ratings("tt0033467").await {
                    Ok(Some(_)) => Ok(Json(TestResponse {
                        ok: true,
                        detail: "OMDb accepted the key".into(),
                    })),
                    Ok(None) => Ok(Json(TestResponse {
                        ok: false,
                        detail: "OMDb returned a 'not found' for the test lookup".into(),
                    })),
                    Err(e) => Ok(Json(TestResponse {
                        ok: false,
                        detail: format!("OMDb rejected: {e:#}"),
                    })),
                },
                Err(e) => Ok(Json(TestResponse {
                    ok: false,
                    detail: format!("OMDb key not usable: {e:#}"),
                })),
            }
        }
        "session_hmac" => Ok(Json(TestResponse {
            ok: true,
            detail: "session HMAC is internal; no external test applies".into(),
        })),
        other => Err(ApiError::validation(format!(
            "no test implementation for slot {other}"
        ))),
    }
}

/// Rebuild whatever long-lived client on `AppState` is backed by the slot
/// we just mutated, so the change takes effect without a server restart.
async fn refresh_runtime_client(
    state: &AppState,
    name: &str,
    new_value: Option<&str>,
) -> Result<(), ApiError> {
    match name {
        "tmdb" => {
            let client = match new_value {
                Some(v) => Some(TmdbClient::new(v).map_err(ApiError::Internal)?),
                None => None,
            };
            state.set_tmdb(client).await;
            Ok(())
        }
        "tvdb" => {
            let client = match new_value {
                Some(v) => Some(TvdbClient::new(v, None).map_err(ApiError::Internal)?),
                None => None,
            };
            state.set_tvdb(client).await;
            Ok(())
        }
        "anilist" => {
            // AniList always has a client (unauthenticated mode works);
            // swapping in a token-bearing one just upgrades the rate
            // limit. Clearing the slot drops back to unauthenticated.
            let client = match new_value {
                Some(v) => AniListClient::with_token(v).map_err(ApiError::Internal)?,
                None => AniListClient::unauthenticated().map_err(ApiError::Internal)?,
            };
            state.set_anilist(Some(client)).await;
            Ok(())
        }
        "opensubtitles" => {
            let client = match new_value {
                Some(v) => {
                    let creds = OpenSubtitlesCreds::parse(v)
                        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
                    Some(OpenSubtitlesClient::new(creds).map_err(ApiError::Internal)?)
                }
                None => None,
            };
            state.set_opensubtitles(client).await;
            Ok(())
        }
        "trakt" => {
            let client = match new_value {
                Some(v) => {
                    let creds = TraktCreds::parse(v)
                        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
                    Some(TraktClient::from_creds(creds).map_err(ApiError::Internal)?)
                }
                None => None,
            };
            state.set_trakt(client).await;
            Ok(())
        }
        "omdb" => {
            let client = match new_value {
                Some(v) => Some(
                    chimpflix_metadata::OmdbClient::new(v).map_err(ApiError::Internal)?,
                ),
                None => None,
            };
            state.set_omdb(client).await;
            Ok(())
        }
        // session_hmac is rejected upstream so we never get here for it.
        _ => Ok(()),
    }
}

async fn slot_view(state: &AppState, spec: &SlotSpec) -> Result<Json<SlotView>, ApiError> {
    // Refresh metadata from the DB instead of synthesizing it — keeps
    // updated_at exact and avoids drift.
    let stored = queries::vault_list_metadata(&state.pool, &state.vault)
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .find(|s| s.name == spec.name);
    Ok(Json(SlotView {
        name: spec.name.into(),
        display_name: spec.display_name.into(),
        description: spec.description.into(),
        managed: spec.managed,
        stored,
    }))
}
