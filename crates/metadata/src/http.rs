//! Shared HTTP helpers for the metadata agents.
//!
//! Every metadata client (TMDB, AniList, Trakt, OMDb, TVDB, TVMaze,
//! OpenSubtitles) makes outbound `reqwest::get(...).json()` /
//! `.text()` / `.bytes()` calls. The default reqwest helpers read the
//! full response body into memory before deserializing — a hostile
//! or broken upstream returning a 10 GB body would hang the worker
//! and exhaust RAM.
//!
//! These helpers stream the body and bail the moment the accumulator
//! crosses `max_bytes`. The per-client `Client::builder().timeout(...)`
//! already bounds wall-clock; this bounds memory.
//!
//! See WEEK 1 #11 in `docs/PUBLIC_RELEASE_HARDENING.md`.

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt as _;
use serde::de::DeserializeOwned;

/// Default cap for metadata JSON / text responses (4 MiB). TMDB / AniList
/// / Trakt list responses come back at a few hundred KiB at worst even
/// for prolific actors / huge calendars.
pub const DEFAULT_METADATA_BYTES: u64 = 4 * 1024 * 1024;

/// Stream the response body, accumulating up to `max_bytes`. Returns
/// the raw bytes or an error labelled with `context_label` when the
/// cap is exceeded or a chunk read fails.
pub async fn bounded_bytes(
    resp: reqwest::Response,
    max_bytes: u64,
    context_label: &str,
) -> Result<Bytes> {
    // content-length pre-check is cheap insurance against an honest
    // server promising to send a huge payload.
    if let Some(len) = resp.content_length() {
        if len > max_bytes {
            anyhow::bail!("{context_label}: content-length {len} > cap {max_bytes}");
        }
    }
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("{context_label}: read chunk"))?;
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 > max_bytes {
            anyhow::bail!(
                "{context_label}: response exceeded cap mid-stream ({} bytes > {} max)",
                buf.len(),
                max_bytes,
            );
        }
    }
    Ok(Bytes::from(buf))
}

/// Streamed equivalent of `resp.text()` with a byte cap.
pub async fn bounded_text(
    resp: reqwest::Response,
    max_bytes: u64,
    context_label: &str,
) -> Result<String> {
    let bytes = bounded_bytes(resp, max_bytes, context_label).await?;
    String::from_utf8(bytes.to_vec())
        .with_context(|| format!("{context_label}: response was not valid UTF-8"))
}

/// Streamed equivalent of `resp.json()` with a byte cap.
pub async fn bounded_json<T: DeserializeOwned>(
    resp: reqwest::Response,
    max_bytes: u64,
    context_label: &str,
) -> Result<T> {
    let bytes = bounded_bytes(resp, max_bytes, context_label).await?;
    serde_json::from_slice(&bytes).with_context(|| format!("{context_label}: JSON parse"))
}
