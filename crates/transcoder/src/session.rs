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
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::FfmpegConfig;

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
) -> Result<Child> {
    let manifest = out_dir.join("index.m3u8");
    let segment_pattern = out_dir.join("seg-%03d.ts");
    let start_seconds = (start_position_ms.max(0) as f64) / 1000.0;

    let scale = format!("scale=-2:'min({TARGET_HEIGHT},ih)'");

    let mut cmd = Command::new(&cfg.ffmpeg);
    cmd.arg("-y")
        .args(["-loglevel", "warning"])
        .args(["-ss", &format!("{start_seconds:.3}")])
        .arg("-i")
        .arg(input)
        .args(["-c:v", "libx264"])
        .args(["-preset", "veryfast"])
        .args(["-crf", "23"])
        .args(["-pix_fmt", "yuv420p"])
        .args(["-profile:v", "main"])
        .args(["-level", "4.0"])
        .args(["-vf", &scale])
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
        "spawning ffmpeg"
    );

    let child = cmd
        .spawn()
        .with_context(|| format!("spawn ffmpeg ({}) for transcode session", cfg.ffmpeg))?;

    Ok(child)
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
