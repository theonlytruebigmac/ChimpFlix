//! HLS transcode session manager.
//!
//! v0.1 scope: single-variant (720p H.264 + AAC) software transcode,
//! spawned via ffmpeg subprocess. Each session has its own output
//! directory under the configured cache root. Sessions are kept alive
//! by recent access; the reaper kills idle ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use chimpflix_common::now_ms;
use serde::Serialize;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::FfmpegConfig;

/// Plain-data snapshot of a transcode session. Returned by
/// `TranscodeManager::list_sessions` so callers can build admin/dashboard
/// views without holding the sessions lock or peeking at `Session`'s
/// non-serializable internals.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub user_id: i64,
    pub media_file_id: i64,
    pub start_position_ms: i64,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    pub last_seen_at: i64,
}

const SESSION_ID_BYTES: usize = 16;
const VARIANT_NAME: &str = "v1";
const TARGET_HEIGHT: u32 = 720;
const TARGET_VIDEO_BITRATE_BPS: u64 = 2_500_000;
const HLS_SEGMENT_DURATION_S: u32 = 6;

#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub media_file_id: i64,
    pub output_dir: PathBuf,
    pub start_position_ms: i64,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    last_seen: AtomicI64,
    _child: Mutex<Child>,
}

impl Session {
    pub fn touch(&self) {
        self.last_seen.store(now_ms(), Ordering::Relaxed);
    }

    pub fn last_seen(&self) -> i64 {
        self.last_seen.load(Ordering::Relaxed)
    }

    pub fn variant_name() -> &'static str {
        VARIANT_NAME
    }

    /// Synthesize the master playlist for this session.
    pub fn master_playlist(&self) -> String {
        // Codecs string is a reasonable assumption for our fixed ladder:
        //   avc1.4d401f → H.264 Main @ Level 4.0
        //   mp4a.40.2  → AAC LC
        format!(
            "#EXTM3U\n\
             #EXT-X-VERSION:3\n\
             #EXT-X-STREAM-INF:BANDWIDTH={bw},RESOLUTION=1280x{h},CODECS=\"avc1.4d401f,mp4a.40.2\"\n\
             {variant}/index.m3u8\n",
            bw = TARGET_VIDEO_BITRATE_BPS + 200_000,
            h = TARGET_HEIGHT,
            variant = VARIANT_NAME,
        )
    }
}

#[derive(Clone)]
pub struct TranscodeManager {
    inner: Arc<Inner>,
}

struct Inner {
    cache_root: PathBuf,
    ffmpeg: FfmpegConfig,
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl TranscodeManager {
    pub fn new(cache_root: PathBuf, ffmpeg: FfmpegConfig) -> Result<Self> {
        std::fs::create_dir_all(&cache_root)
            .with_context(|| format!("create transcode cache dir {}", cache_root.display()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                cache_root,
                ffmpeg,
                sessions: RwLock::new(HashMap::new()),
            }),
        })
    }

    pub async fn start(
        &self,
        media_file_id: i64,
        media_file_path: &Path,
        start_position_ms: i64,
        duration_ms: Option<i64>,
        user_id: i64,
        audio_index: Option<u32>,
        subtitle_index: Option<u32>,
        subtitle_codec: Option<&str>,
    ) -> Result<Arc<Session>> {
        let id = generate_id();
        let session_dir = self.inner.cache_root.join(&id);
        let variant_dir = session_dir.join(VARIANT_NAME);
        tokio::fs::create_dir_all(&variant_dir)
            .await
            .with_context(|| format!("create session dir {}", variant_dir.display()))?;

        let child = spawn_ffmpeg(
            &self.inner.ffmpeg,
            media_file_path,
            &variant_dir,
            start_position_ms,
            audio_index,
            subtitle_index,
            subtitle_codec,
            &id,
        )
        .await?;

        let now = now_ms();
        let session = Arc::new(Session {
            id: id.clone(),
            user_id,
            media_file_id,
            output_dir: session_dir,
            start_position_ms,
            duration_ms,
            created_at: now,
            last_seen: AtomicI64::new(now),
            _child: Mutex::new(child),
        });

        self.inner
            .sessions
            .write()
            .expect("sessions lock")
            .insert(id, session.clone());
        info!(
            session_id = %session.id,
            media_file_id,
            user_id,
            "transcode session started"
        );
        Ok(session)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.inner
            .sessions
            .read()
            .expect("sessions lock")
            .get(id)
            .cloned()
    }

    pub async fn delete(&self, id: &str) -> bool {
        let session = self
            .inner
            .sessions
            .write()
            .expect("sessions lock")
            .remove(id);
        let Some(session) = session else {
            return false;
        };
        let path = session.output_dir.clone();
        drop(session); // Dropping kills the ffmpeg child via kill_on_drop.
        let _ = tokio::fs::remove_dir_all(&path).await;
        info!(session_id = id, "transcode session deleted");
        true
    }

    /// Snapshot of every currently-running session. Used by the admin
    /// dashboard; callers receive plain data and don't share state with
    /// the live `Session` objects.
    pub fn list_sessions(&self) -> Vec<SessionSnapshot> {
        let map = self.inner.sessions.read().expect("sessions lock");
        map.values()
            .map(|s| SessionSnapshot {
                id: s.id.clone(),
                user_id: s.user_id,
                media_file_id: s.media_file_id,
                start_position_ms: s.start_position_ms,
                duration_ms: s.duration_ms,
                created_at: s.created_at,
                last_seen_at: s.last_seen(),
            })
            .collect()
    }

    pub async fn reap_idle(&self, idle_threshold_ms: i64) -> usize {
        let now = now_ms();
        let stale: Vec<String> = {
            let map = self.inner.sessions.read().expect("sessions lock");
            map.iter()
                .filter(|(_, s)| now - s.last_seen() > idle_threshold_ms)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let count = stale.len();
        for id in stale {
            self.delete(&id).await;
        }
        if count > 0 {
            debug!(count, "reaped idle transcode sessions");
        }
        count
    }

    /// Spawn a background task that periodically reaps idle sessions.
    pub fn spawn_reaper(&self, idle_threshold_ms: i64, interval_s: u64) {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval_s));
            tick.tick().await; // skip the immediate tick
            loop {
                tick.tick().await;
                manager.reap_idle(idle_threshold_ms).await;
            }
        });
    }
}

async fn spawn_ffmpeg(
    cfg: &FfmpegConfig,
    input: &Path,
    out_dir: &Path,
    start_position_ms: i64,
    audio_index: Option<u32>,
    subtitle_index: Option<u32>,
    subtitle_codec: Option<&str>,
    session_id: &str,
) -> Result<Child> {
    let manifest = out_dir.join("index.m3u8");
    let segment_pattern = out_dir.join("seg-%03d.ts");
    let start_seconds = (start_position_ms.max(0) as f64) / 1000.0;

    // Two subtitle-burn strategies depending on codec:
    //   * Text-based (subrip/ass/ssa/mov_text/webvtt) — the `subtitles`
    //     filter reads from the file by stream index.
    //   * Picture-based (hdmv_pgs_subtitle/dvd_subtitle/dvb_subtitle/
    //     vobsub) — the `subtitles` filter doesn't accept them; instead
    //     we map the subtitle stream into a filter graph and overlay it
    //     onto the video.
    //
    // When subtitle_codec is unknown we default to the text path; ffmpeg
    // will fail loudly and the captured stderr will tell us what to do
    // next.
    let mut cmd = Command::new(&cfg.ffmpeg);
    cmd.arg("-y")
        .args(["-loglevel", "warning"])
        .args(["-ss", &format!("{start_seconds:.3}")])
        .arg("-i")
        .arg(input);

    let kind = subtitle_kind(subtitle_codec);
    let needs_filter_complex = matches!(kind, SubtitleKind::Picture);
    let burn_text = matches!(kind, SubtitleKind::Text);
    let scale = format!("scale=-2:'min({TARGET_HEIGHT},ih)'");

    if needs_filter_complex {
        // Picture-based: overlay subtitle stream onto video before scale.
        // `0:v:0` is the primary video; `0:s:N` is the Nth subtitle
        // stream in the source. Result is named [v] and mapped to the
        // encoder's video input.
        let si = subtitle_index.unwrap_or(0);
        let fc = format!(
            "[0:v:0][0:s:{si}]overlay[vs];[vs]{scale}[v]"
        );
        cmd.args(["-filter_complex", &fc])
            .args(["-map", "[v]"]);
        // Audio map (explicit if caller picked, else default first audio).
        if let Some(ai) = audio_index {
            cmd.args(["-map", &format!("0:a:{ai}")]);
        } else {
            cmd.args(["-map", "0:a:0?"]);
        }
    } else {
        // Text-based or no subtitle burn — single -vf chain.
        let mut vf = scale.clone();
        if burn_text {
            if let Some(si) = subtitle_index {
                let escaped = escape_for_filter(&input.to_string_lossy());
                vf = format!("{vf},subtitles=filename='{escaped}':si={si}");
            }
        }
        cmd.args(["-vf", &vf]);
        if let Some(ai) = audio_index {
            cmd.args(["-map", "0:v:0"])
                .args(["-map", &format!("0:a:{ai}")]);
        }
    }

    cmd.args(["-c:v", "libx264"])
        .args(["-preset", "veryfast"])
        .args(["-crf", "23"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-profile:v", "main"])
        .args(["-level", "4.0"])
        .args(["-c:a", "aac"])
        .args(["-b:a", "192k"])
        .args(["-ac", "2"])
        .args(["-f", "hls"])
        .args(["-hls_time", &HLS_SEGMENT_DURATION_S.to_string()])
        .args(["-hls_segment_filename"])
        .arg(&segment_pattern)
        .args(["-hls_flags", "independent_segments+temp_file"])
        .arg(&manifest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    debug!(
        ffmpeg = %cfg.ffmpeg,
        input = %input.display(),
        out_dir = %out_dir.display(),
        start_s = start_seconds,
        subtitle_index = ?subtitle_index,
        subtitle_codec = ?subtitle_codec,
        subtitle_kind = ?kind,
        "spawning ffmpeg"
    );

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn ffmpeg ({}) for transcode session", cfg.ffmpeg))?;

    // Drain stderr in the background. ffmpeg writes warnings + the final
    // exit reason here; without a reader the pipe blocks once its kernel
    // buffer fills (~64 KB) which silently stalls ffmpeg. We log at warn
    // so transcode failures show up in the admin Logs page.
    if let Some(stderr) = child.stderr.take() {
        let session_id = session_id.to_string();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                warn!(session_id = %session_id, ffmpeg = %line, "transcoder");
            }
        });
    }

    Ok(child)
}

#[derive(Debug, Clone, Copy)]
enum SubtitleKind {
    /// No subtitle burn requested.
    None,
    /// Text-based (subrip/ass/mov_text/webvtt) — `subtitles=` filter.
    Text,
    /// Picture-based (PGS/DVD/VobSub) — `overlay` via filter_complex.
    Picture,
}

fn subtitle_kind(codec: Option<&str>) -> SubtitleKind {
    let Some(c) = codec else {
        return SubtitleKind::None;
    };
    let c = c.to_ascii_lowercase();
    match c.as_str() {
        "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text" => SubtitleKind::Text,
        "hdmv_pgs_subtitle" | "pgs" | "dvd_subtitle" | "dvdsub" | "dvb_subtitle" | "vobsub"
        | "xsub" => SubtitleKind::Picture,
        // Unknown — try text first; if ffmpeg complains we'll see it in
        // the captured stderr.
        _ => SubtitleKind::Text,
    }
}

/// Escape a path so it survives being interpolated inside ffmpeg's
/// single-quoted filter argument. ffmpeg's filtergraph parser is finicky
/// about `'`, `:`, `\`, `[`, `]`, and `,`.
fn escape_for_filter(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' | '\'' | ':' | '[' | ']' | ',' | ';' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn generate_id() -> String {
    let mut buf = [0u8; SESSION_ID_BYTES];
    if let Err(e) = fill_random(&mut buf) {
        // Catastrophic OS RNG failure. Fall back to time-based ID; we
        // log loudly because this should never happen in practice.
        warn!(error = %e, "RNG failure during session id generation; using time-based fallback");
        let now = now_ms();
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = (now >> ((i % 8) * 8)) as u8;
        }
    }
    hex::encode(buf)
}

fn fill_random(buf: &mut [u8]) -> Result<()> {
    use rand_core::{OsRng, RngCore};
    let mut rng = OsRng;
    rng.fill_bytes(buf);
    Ok(())
}
