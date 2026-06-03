"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type DashboardSession,
  type HevcMode,
  type ServerSettings,
  type SubtitleHealth,
  type TonemapAlgorithm,
  type TranscodeHealth,
  type TranscoderBackgroundPreset,
  type TranscoderCapabilities,
  type TranscoderEncoderPreset,
  type TranscoderHwAccel,
  type TranscoderHwStrictness,
} from "@/lib/chimpflix-api";

const HEVC_MODES: ReadonlyArray<{ value: HevcMode; label: string; hint: string }> = [
  {
    value: "off",
    label: "Off — always H.264",
    hint: "Default. Maximum browser compatibility — every client can decode H.264.",
  },
  {
    value: "when_client_supports",
    label: "When the client supports it",
    hint: "Safe-conservative: HEVC for Safari + some Edge / Chrome builds, H.264 otherwise.",
  },
  {
    value: "always",
    label: "Always (Safari only)",
    hint: "Forces HEVC for every transcode. Breaks Firefox + many Chrome builds. Set only on Safari-only deployments.",
  },
];

const BACKGROUND_PRESETS: ReadonlyArray<{
  value: TranscoderBackgroundPreset;
  label: string;
}> = [
  { value: "ultrafast", label: "ultrafast (lowest CPU, largest output)" },
  { value: "superfast", label: "superfast" },
  { value: "veryfast", label: "veryfast (default)" },
  { value: "faster", label: "faster" },
  { value: "fast", label: "fast" },
  { value: "medium", label: "medium" },
  { value: "slow", label: "slow" },
  { value: "slower", label: "slower (smallest output, slowest)" },
];

const TONEMAP_ALGOS: ReadonlyArray<{
  value: TonemapAlgorithm;
  label: string;
  hint: string;
}> = [
  { value: "hable", label: "Hable (filmic)", hint: "Filmic curve — default, balanced." },
  { value: "reinhard", label: "Reinhard", hint: "Classic, slightly washed-out highlights." },
  { value: "mobius", label: "Mobius", hint: "Preserves bright highlights — good for HDR10." },
  { value: "bt2390", label: "BT.2390", hint: "ITU reference — broadcast-style." },
  { value: "clip", label: "Clip", hint: "Hard clip — fastest, blown highlights." },
  { value: "linear", label: "Linear", hint: "Linear scale — washed-out, debug-only." },
];

const HW_ACCEL_OPTIONS: ReadonlyArray<{
  value: TranscoderHwAccel;
  label: string;
  requires: string | null;
}> = [
  { value: "auto", label: "Auto", requires: null },
  { value: "nvenc", label: "NVENC (NVIDIA)", requires: "h264_nvenc" },
  { value: "vaapi", label: "VAAPI", requires: "h264_vaapi" },
  { value: "qsv", label: "Quick Sync", requires: "h264_qsv" },
  { value: "videotoolbox", label: "VideoToolbox", requires: "h264_videotoolbox" },
  { value: "amf", label: "AMF", requires: "h264_amf" },
  { value: "none", label: "None (software / libx264)", requires: null },
];

/// The H.264 capability chips shown in the detected-hardware card. Each
/// maps a friendly label to the encoder token ffmpeg reports; the chip
/// renders "available" (green tick) when the token was probed at boot.
const H264_CAP_CHIPS: ReadonlyArray<{ label: string; encoder: string }> = [
  { label: "NVENC", encoder: "h264_nvenc" },
  { label: "VAAPI", encoder: "h264_vaapi" },
  { label: "Quick Sync", encoder: "h264_qsv" },
  { label: "VideoToolbox", encoder: "h264_videotoolbox" },
  { label: "AMF", encoder: "h264_amf" },
];

interface Props {
  capabilities: TranscoderCapabilities;
  cacheRoot: string;
  settings: ServerSettings;
}

function CheckIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
      <path d="M5 12l5 5 9-11" />
    </svg>
  );
}

export function AdminTranscoderClient({
  capabilities,
  cacheRoot,
  settings,
}: Props) {
  // See AdminGeneralForm for rationale — keep a local baseline so
  // save success updates it via `setBaseline` rather than mutating
  // the `settings` prop in place.
  const [baseline, setBaseline] = useState({
    transcoder_hw_accel: settings.transcoder_hw_accel,
    transcoder_max_concurrent: settings.transcoder_max_concurrent,
    transcoder_max_cpu_concurrent: settings.transcoder_max_cpu_concurrent,
    transcoder_quality_ceiling_kbps:
      settings.transcoder_quality_ceiling_kbps ?? null,
    transcoder_encoder_preset: settings.transcoder_encoder_preset,
    transcoder_hw_strictness: settings.transcoder_hw_strictness,
    transcoder_background_preset: settings.transcoder_background_preset,
    transcoder_max_background_concurrent:
      settings.transcoder_max_background_concurrent,
    job_workers: settings.job_workers,
    transcoder_hdr_tonemap_enabled: settings.transcoder_hdr_tonemap_enabled,
    transcoder_hdr_tonemap_algo: settings.transcoder_hdr_tonemap_algo,
    transcoder_burn_ass_subtitles: settings.transcoder_burn_ass_subtitles,
    transcoder_two_pass_loudnorm: settings.transcoder_two_pass_loudnorm,
    transcoder_hevc_encoding_mode: (settings.transcoder_hevc_encoding_mode ??
      "off") as HevcMode,
    transcoder_gpu_device: settings.transcoder_gpu_device ?? "auto",
  });
  const [hwAccel, setHwAccel] = useState<TranscoderHwAccel>(
    baseline.transcoder_hw_accel,
  );
  const [maxConcurrent, setMaxConcurrent] = useState(
    baseline.transcoder_max_concurrent,
  );
  const [maxCpuConcurrent, setMaxCpuConcurrent] = useState(
    baseline.transcoder_max_cpu_concurrent,
  );
  const [ceiling, setCeiling] = useState<number | "">(
    baseline.transcoder_quality_ceiling_kbps ?? "",
  );
  const [encoderPreset, setEncoderPreset] = useState<TranscoderEncoderPreset>(
    baseline.transcoder_encoder_preset,
  );
  const [hwStrictness, setHwStrictness] = useState<TranscoderHwStrictness>(
    baseline.transcoder_hw_strictness,
  );
  const [backgroundPreset, setBackgroundPreset] = useState<TranscoderBackgroundPreset>(
    baseline.transcoder_background_preset,
  );
  const [maxBackgroundConcurrent, setMaxBackgroundConcurrent] = useState(
    baseline.transcoder_max_background_concurrent,
  );
  const [jobWorkers, setJobWorkers] = useState(baseline.job_workers);
  const [tonemapEnabled, setTonemapEnabled] = useState(
    baseline.transcoder_hdr_tonemap_enabled,
  );
  const [tonemapAlgo, setTonemapAlgo] = useState<TonemapAlgorithm>(
    baseline.transcoder_hdr_tonemap_algo,
  );
  const [burnAssSubtitles, setBurnAssSubtitles] = useState(
    baseline.transcoder_burn_ass_subtitles,
  );
  const [twoPassLoudnorm, setTwoPassLoudnorm] = useState(
    baseline.transcoder_two_pass_loudnorm,
  );
  const [hevcMode, setHevcMode] = useState<HevcMode>(
    baseline.transcoder_hevc_encoding_mode,
  );
  const [gpuDevice, setGpuDevice] = useState<string>(
    baseline.transcoder_gpu_device,
  );
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // Detected hardware is server-rendered into the `capabilities` prop,
  // but the "Re-probe" button can refresh it at runtime without a page
  // reload — so the displayed chips read from this local copy.
  const [caps, setCaps] = useState<TranscoderCapabilities>(capabilities);
  const [reprobing, setReprobing] = useState(false);
  const [reprobedAt, setReprobedAt] = useState<number | null>(null);
  const [active, setActive] = useState<DashboardSession[]>([]);
  // Start at 0 — first dashboard response below replaces it with
  // server.now_ms. Initializing from Date.now() in useState would call
  // an impure function during render, which React's strict-mode rules
  // forbid (and which causes non-deterministic re-renders).
  const [nowMs, setNowMs] = useState<number>(0);

  // Active transcodes are pushed live over /api/v1/ws. We also hit the
  // dashboard endpoint once on mount to populate the list before the first
  // WS frame arrives (the server sends an initial snapshot on connect, but
  // open-handshake latency means the table is otherwise empty for a beat).
  useEffect(() => {
    let cancelled = false;
    adminApi
      .dashboard()
      .then((d) => {
        if (cancelled) return;
        setActive(d.active_transcodes);
        setNowMs(d.server.now_ms);
      })
      .catch((e: unknown) => {
        // Initial snapshot fetch failed — the WS will (re)populate
        // active_transcodes once it connects, but surface the failure
        // so a misconfigured admin auth or down server isn't silent.
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      });

    if (typeof window === "undefined") return () => {};
    let socket: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    function connect() {
      if (cancelled) return;
      const proto = window.location.protocol === "https:" ? "wss" : "ws";
      socket = new WebSocket(
        `${proto}://${window.location.host}/api/v1/ws`,
      );
      socket.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data as string) as {
            type?: string;
            active?: DashboardSession[];
          };
          if (msg.type === "sessions" && Array.isArray(msg.active)) {
            setActive(msg.active);
            setNowMs(Date.now());
          }
        } catch {
          // Ignore unrelated frames.
        }
      };
      socket.onclose = () => {
        if (cancelled) return;
        reconnectTimer = setTimeout(connect, 5_000);
      };
      socket.onerror = () => socket?.close();
    }
    connect();
    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      socket?.close();
    };
  }, []);

  async function stopSession(id: string) {
    try {
      await adminApi.stopSession(id);
      // The WS feed will push the updated list within ~1s of the kill.
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // Re-run ffmpeg hardware detection server-side and refresh the
  // displayed chips. Useful after a driver upgrade or GPU change without
  // restarting the server. The server swaps the fresh result into the
  // live encoder-selection path too, so this is not cosmetic.
  async function reprobe() {
    setReprobing(true);
    setError(null);
    try {
      const res = await adminApi.transcoder.reprobeCapabilities();
      setCaps(res.capabilities);
      setReprobedAt(Date.now());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setReprobing(false);
    }
  }

  // Per-field dirty flags. Drives the cf-changed input outline
  // and the savebar dirtyCount + summary.
  const dirtyFields: Record<string, boolean> = {
    "Hardware acceleration": hwAccel !== baseline.transcoder_hw_accel,
    "Max concurrent": maxConcurrent !== baseline.transcoder_max_concurrent,
    "Max CPU-only":
      maxCpuConcurrent !== baseline.transcoder_max_cpu_concurrent,
    "Quality ceiling":
      (ceiling === "" ? null : Number(ceiling)) !==
      baseline.transcoder_quality_ceiling_kbps,
    "Live encoder preset":
      encoderPreset !== baseline.transcoder_encoder_preset,
    "Fallback strictness": hwStrictness !== baseline.transcoder_hw_strictness,
    "Background preset":
      backgroundPreset !== baseline.transcoder_background_preset,
    "Background slots":
      maxBackgroundConcurrent !==
      baseline.transcoder_max_background_concurrent,
    "Job workers": jobWorkers !== baseline.job_workers,
    "HDR tonemapping":
      tonemapEnabled !== baseline.transcoder_hdr_tonemap_enabled,
    "Tonemap algorithm":
      tonemapAlgo !== baseline.transcoder_hdr_tonemap_algo,
    "Burn-in subtitles":
      burnAssSubtitles !== baseline.transcoder_burn_ass_subtitles,
    "Two-pass loudnorm":
      twoPassLoudnorm !== baseline.transcoder_two_pass_loudnorm,
    "GPU device": gpuDevice !== baseline.transcoder_gpu_device,
    "HEVC encoding": hevcMode !== baseline.transcoder_hevc_encoding_mode,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, isDirty]) => isDirty)
    .map(([label]) => label);
  const dirtyCount = dirtyLabels.length;

  async function saveSettings() {
    // Guard against concurrent saves from a double-click.
    if (saving) return;
    setSaving(true);
    setError(null);
    const patch = {
      transcoder_hw_accel: hwAccel,
      transcoder_max_concurrent: maxConcurrent,
      transcoder_max_cpu_concurrent: maxCpuConcurrent,
      transcoder_quality_ceiling_kbps: ceiling === "" ? null : Number(ceiling),
      transcoder_encoder_preset: encoderPreset,
      transcoder_hw_strictness: hwStrictness,
      transcoder_background_preset: backgroundPreset,
      transcoder_max_background_concurrent: maxBackgroundConcurrent,
      job_workers: jobWorkers,
      transcoder_hdr_tonemap_enabled: tonemapEnabled,
      transcoder_hdr_tonemap_algo: tonemapAlgo,
      transcoder_burn_ass_subtitles: burnAssSubtitles,
      transcoder_two_pass_loudnorm: twoPassLoudnorm,
      transcoder_hevc_encoding_mode: hevcMode,
      transcoder_gpu_device: gpuDevice,
    };
    try {
      // Capture the server response so the baseline reflects server-persisted
      // values (the backend may clamp or normalize individual fields).
      const { settings: saved } = await adminApi.settings.patch(patch);
      const savedHevc = (saved.transcoder_hevc_encoding_mode ?? "off") as HevcMode;
      const savedGpu = saved.transcoder_gpu_device ?? "auto";
      setBaseline({
        transcoder_hw_accel: saved.transcoder_hw_accel,
        transcoder_max_concurrent: saved.transcoder_max_concurrent,
        transcoder_max_cpu_concurrent: saved.transcoder_max_cpu_concurrent,
        transcoder_quality_ceiling_kbps:
          saved.transcoder_quality_ceiling_kbps ?? null,
        transcoder_encoder_preset: saved.transcoder_encoder_preset,
        transcoder_hw_strictness: saved.transcoder_hw_strictness,
        transcoder_background_preset: saved.transcoder_background_preset,
        transcoder_max_background_concurrent:
          saved.transcoder_max_background_concurrent,
        job_workers: saved.job_workers,
        transcoder_hdr_tonemap_enabled: saved.transcoder_hdr_tonemap_enabled,
        transcoder_hdr_tonemap_algo: saved.transcoder_hdr_tonemap_algo,
        transcoder_burn_ass_subtitles: saved.transcoder_burn_ass_subtitles,
        transcoder_two_pass_loudnorm: saved.transcoder_two_pass_loudnorm,
        transcoder_hevc_encoding_mode: savedHevc,
        transcoder_gpu_device: savedGpu,
      });
      // Sync control state from server-accepted values so UI reflects what
      // was actually stored (e.g. if the server clamped a numeric field).
      setHwAccel(saved.transcoder_hw_accel);
      setMaxConcurrent(saved.transcoder_max_concurrent);
      setMaxCpuConcurrent(saved.transcoder_max_cpu_concurrent);
      setCeiling(saved.transcoder_quality_ceiling_kbps ?? "");
      setEncoderPreset(saved.transcoder_encoder_preset);
      setHwStrictness(saved.transcoder_hw_strictness);
      setBackgroundPreset(saved.transcoder_background_preset);
      setMaxBackgroundConcurrent(saved.transcoder_max_background_concurrent);
      setJobWorkers(saved.job_workers);
      setTonemapEnabled(saved.transcoder_hdr_tonemap_enabled);
      setTonemapAlgo(saved.transcoder_hdr_tonemap_algo);
      setBurnAssSubtitles(saved.transcoder_burn_ass_subtitles);
      setTwoPassLoudnorm(saved.transcoder_two_pass_loudnorm);
      setHevcMode(savedHevc);
      setGpuDevice(savedGpu);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  function discardChanges() {
    setHwAccel(baseline.transcoder_hw_accel);
    setMaxConcurrent(baseline.transcoder_max_concurrent);
    setMaxCpuConcurrent(baseline.transcoder_max_cpu_concurrent);
    setCeiling(baseline.transcoder_quality_ceiling_kbps ?? "");
    setEncoderPreset(baseline.transcoder_encoder_preset);
    setHwStrictness(baseline.transcoder_hw_strictness);
    setBackgroundPreset(baseline.transcoder_background_preset);
    setMaxBackgroundConcurrent(baseline.transcoder_max_background_concurrent);
    setJobWorkers(baseline.job_workers);
    setTonemapEnabled(baseline.transcoder_hdr_tonemap_enabled);
    setTonemapAlgo(baseline.transcoder_hdr_tonemap_algo);
    setBurnAssSubtitles(baseline.transcoder_burn_ass_subtitles);
    setTwoPassLoudnorm(baseline.transcoder_two_pass_loudnorm);
    setHevcMode(baseline.transcoder_hevc_encoding_mode);
    setGpuDevice(baseline.transcoder_gpu_device);
  }

  const tonemapHint =
    TONEMAP_ALGOS.find((a) => a.value === tonemapAlgo)?.hint ?? "";
  const hevcHint = HEVC_MODES.find((m) => m.value === hevcMode)?.hint ?? "";
  const autoLabel = pickAutoLabel(caps.h264_encoders);
  // Per-hwaccel decode lists for the "Hardware decode" chip rows. Only
  // backends with at least one probed codec are shown.
  const decodeBackends: ReadonlyArray<[string, string[]]> = [
    ["NVDEC (cuda)", caps.decoders.cuda],
    ["VAAPI", caps.decoders.vaapi],
    ["Quick Sync (qsv)", caps.decoders.qsv],
    ["VideoToolbox", caps.decoders.videotoolbox],
  ];
  const decodeBackendsWithCodecs = decodeBackends.filter(
    ([, codecs]) => codecs.length > 0,
  );
  const usingHw =
    caps.h264_encoders.length > 0 &&
    (hwAccel === "auto"
      ? autoLabel !== "Software (libx264)"
      : hwAccel !== "none" &&
        caps.h264_encoders.includes(`h264_${hwAccel}`));

  return (
    <div>
      {error && (
        <div role="alert" aria-live="assertive" className="cf-banner cf-err">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <circle cx="12" cy="12" r="9" />
            <path d="M12 8v4M12 16v.5" />
          </svg>
          <div>{error}</div>
        </div>
      )}

      {/* ── Detected hardware ─────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Detected hardware</div>
            <div className="cf-sub">
              Probed via{" "}
              <span className="cf-mono">ffmpeg -encoders</span>
              {caps.ffmpeg_version ? ` · ${caps.ffmpeg_version}` : ""}
              {reprobedAt
                ? ` · re-probed ${new Date(reprobedAt).toLocaleTimeString()}`
                : " at boot"}
            </div>
          </div>
          <div className="cf-head-aside">
            <button
              type="button"
              className="cf-btn cf-sm"
              onClick={reprobe}
              disabled={reprobing}
              title="Re-run ffmpeg hardware detection without restarting the server (use after a driver or GPU change)"
            >
              {reprobing ? "Re-probing…" : "Re-probe"}
            </button>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div className="cf-field-label">H.264 acceleration</div>
          <div className="cf-flex cf-wrap cf-gap8" style={{ marginBottom: 14 }}>
            {H264_CAP_CHIPS.map((chip) => {
              const ok = caps.h264_encoders.includes(chip.encoder);
              return (
                <span
                  key={chip.encoder}
                  className={"cf-pill" + (ok ? " cf-ok" : "")}
                >
                  {ok && <CheckIcon />}
                  {chip.label}
                </span>
              );
            })}
          </div>
          <div className="cf-field-label">HEVC / others</div>
          <div className="cf-flex cf-wrap cf-gap8">
            <span
              className={
                "cf-pill" +
                (caps.hevc_encoders.includes("hevc_nvenc") ? " cf-ok" : "")
              }
            >
              {caps.hevc_encoders.includes("hevc_nvenc") && <CheckIcon />}
              NVENC HEVC
            </span>
            <span className="cf-pill">libx265 (software)</span>
            {usingHw && caps.gpu_devices.length === 0 && (
              <span className="cf-pill cf-ok">
                <CheckIcon />
                GPU active
              </span>
            )}
          </div>

          {/* GPU devices — one chip per enumerated card. */}
          {caps.gpu_devices.length > 0 && (
            <>
              <div className="cf-field-label" style={{ marginTop: 14 }}>
                GPU devices
              </div>
              <div className="cf-flex cf-wrap cf-gap8">
                {caps.gpu_devices.map((d) => (
                  <span key={d.value} className="cf-pill cf-ok">
                    <CheckIcon />
                    {d.name}
                  </span>
                ))}
              </div>
            </>
          )}

          {/* Hardware decode — per-hwaccel list of source codecs the
              runtime probe confirmed this host can decode on the GPU.
              Rendered as cf-tag chips per codec under each backend. */}
          {decodeBackendsWithCodecs.length > 0 && (
            <>
              <div className="cf-field-label" style={{ marginTop: 14 }}>
                Hardware decode
              </div>
              <div
                className="cf-flex cf-wrap cf-gap8"
                style={{ alignItems: "center" }}
              >
                {decodeBackendsWithCodecs.map(([backend, codecs]) => (
                  <span
                    key={backend}
                    className="cf-flex cf-gap8"
                    style={{ alignItems: "center" }}
                  >
                    <span className="cf-pill cf-ok">
                      <CheckIcon />
                      {backend}
                    </span>
                    {codecs.map((codec) => (
                      <span key={codec} className="cf-tag">
                        {codec}
                      </span>
                    ))}
                  </span>
                ))}
              </div>
            </>
          )}
          {usingHw && (
            <div className="cf-banner cf-ok" style={{ margin: "16px 0 0" }}>
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z" />
              </svg>
              <div>
                Hardware encoding is active — CPU stays free during transcodes.
                Software fallback only kicks in if the GPU is saturated.
              </div>
            </div>
          )}
        </div>
      </div>

      {/* ── Acceleration ──────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Acceleration</div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Hardware acceleration</div>
              <div className="cf-row-help">
                {hwAccel === "auto" ? (
                  <>
                    Auto will pick:{" "}
                    <span className="cf-mono">{autoLabel}</span>. Greyed options
                    weren&apos;t detected on this host.
                  </>
                ) : (
                  "Greyed options weren't detected on this host."
                )}
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["Hardware acceleration"] ? " cf-changed" : "")
                }
                value={hwAccel}
                onChange={(e) => setHwAccel(e.target.value as TranscoderHwAccel)}
              >
                {HW_ACCEL_OPTIONS.map((opt) => {
                  const available =
                    opt.requires == null ||
                    caps.h264_encoders.includes(opt.requires);
                  return (
                    <option
                      key={opt.value}
                      value={opt.value}
                      disabled={!available && opt.value !== hwAccel}
                    >
                      {opt.label}
                      {!available && opt.requires != null
                        ? " — not detected"
                        : ""}
                    </option>
                  );
                })}
              </select>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Fallback strictness</div>
              <div className="cf-row-help">
                What to do when the GPU is busy or a codec isn&apos;t
                HW-supported.
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["Fallback strictness"] ? " cf-changed" : "")
                }
                value={hwStrictness}
                onChange={(e) =>
                  setHwStrictness(e.target.value as TranscoderHwStrictness)
                }
              >
                <option value="auto">Prefer HW, fall back to software</option>
                <option value="prefer_hw">Prefer HW (warn on fallback)</option>
                <option value="require_hw">HW only — fail if unavailable</option>
              </select>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">GPU device</div>
              <div className="cf-row-help">
                {caps.gpu_devices.length === 0
                  ? "Pin to a specific GPU when more than one is present. No multi-GPU devices detected; only Auto is offered."
                  : "Pin to a specific GPU when more than one is present."}
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["GPU device"] ? " cf-changed" : "")
                }
                value={gpuDevice}
                onChange={(e) => setGpuDevice(e.target.value)}
              >
                <option value="auto">auto</option>
                {caps.gpu_devices.map((d) => (
                  <option key={d.value} value={d.value}>
                    {d.name}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </div>
      </div>

      {/* ── Concurrency ───────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Concurrency</div>
            <div className="cf-sub">
              Slot limits so a busy night doesn&apos;t melt the box.
            </div>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <div className="cf-grid cf-c4">
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Max concurrent</label>
              <input
                type="number"
                min={1}
                max={64}
                className={
                  "cf-input" + (dirtyFields["Max concurrent"] ? " cf-changed" : "")
                }
                value={maxConcurrent}
                onChange={(e) => setMaxConcurrent(Number(e.target.value))}
              />
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Max CPU-only</label>
              <input
                type="number"
                min={1}
                max={16}
                className={
                  "cf-input" + (dirtyFields["Max CPU-only"] ? " cf-changed" : "")
                }
                value={maxCpuConcurrent}
                onChange={(e) => setMaxCpuConcurrent(Number(e.target.value))}
              />
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Background slots</label>
              <input
                type="number"
                min={1}
                max={16}
                className={
                  "cf-input" +
                  (dirtyFields["Background slots"] ? " cf-changed" : "")
                }
                value={maxBackgroundConcurrent}
                onChange={(e) =>
                  setMaxBackgroundConcurrent(Number(e.target.value))
                }
              />
            </div>
            <div className="cf-field" style={{ marginBottom: 0 }}>
              <label className="cf-field-label">Job workers</label>
              <input
                type="number"
                min={1}
                max={16}
                className={
                  "cf-input" + (dirtyFields["Job workers"] ? " cf-changed" : "")
                }
                value={jobWorkers}
                onChange={(e) => setJobWorkers(Number(e.target.value))}
              />
            </div>
          </div>
        </div>
      </div>

      {/* ── Quality ───────────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Quality</div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Live encoder preset</div>
              <div className="cf-row-help">
                Speed vs quality for on-the-fly transcodes.
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["Live encoder preset"] ? " cf-changed" : "")
                }
                value={encoderPreset}
                onChange={(e) =>
                  setEncoderPreset(e.target.value as TranscoderEncoderPreset)
                }
              >
                <option value="speed">Speed (fastest)</option>
                <option value="balanced">Balanced (default)</option>
                <option value="quality">Quality (best)</option>
              </select>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Background preset</div>
              <div className="cf-row-help">
                For unattended optimize jobs — can afford to be slower. Always
                uses libx264 (no GPU).
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["Background preset"] ? " cf-changed" : "")
                }
                value={backgroundPreset}
                onChange={(e) =>
                  setBackgroundPreset(e.target.value as TranscoderBackgroundPreset)
                }
              >
                {BACKGROUND_PRESETS.map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Quality ceiling</div>
              <div className="cf-row-help">
                Cap the max output bitrate. Blank = no cap.
              </div>
            </div>
            <div className="cf-row-control">
              <input
                type="number"
                min={100}
                max={200_000}
                className={
                  "cf-input cf-w-auto" +
                  (dirtyFields["Quality ceiling"] ? " cf-changed" : "")
                }
                style={{ minWidth: 120 }}
                placeholder="kbps"
                value={ceiling}
                onChange={(e) =>
                  setCeiling(e.target.value === "" ? "" : Number(e.target.value))
                }
              />
              <span className="cf-faint">kbps</span>
            </div>
          </div>
        </div>
      </div>

      {/* ── Advanced video ────────────────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Advanced video</div>
            <div className="cf-sub">
              Leave defaults unless you know you need these.
            </div>
          </div>
        </div>
        <div className="cf-card-body">
          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">HDR → SDR tonemapping</div>
              <div className="cf-row-help">
                Convert HDR10 / HLG / Dolby Vision for SDR clients.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={tonemapEnabled}
                aria-label="HDR to SDR tonemapping"
                className={"cf-switch" + (tonemapEnabled ? " cf-on" : "")}
                onClick={() => setTonemapEnabled((v) => !v)}
              />
            </div>
          </div>

          <div
            className="cf-row"
            style={{
              transition: "opacity .2s",
              opacity: tonemapEnabled ? 1 : 0.4,
              pointerEvents: tonemapEnabled ? "auto" : "none",
            }}
          >
            <div className="cf-row-main">
              <div className="cf-row-label">Tonemap algorithm</div>
              <div className="cf-row-help">
                {tonemapHint || "Only used when tonemapping is on."}
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["Tonemap algorithm"] ? " cf-changed" : "")
                }
                value={tonemapAlgo}
                disabled={!tonemapEnabled}
                onChange={(e) =>
                  setTonemapAlgo(e.target.value as TonemapAlgorithm)
                }
              >
                {TONEMAP_ALGOS.map((a) => (
                  <option key={a.value} value={a.value}>
                    {a.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">HEVC encoding</div>
              <div className="cf-row-help">
                {hevcHint ||
                  "HEVC halves bitrate but only Safari plays it reliably."}
              </div>
            </div>
            <div className="cf-row-control">
              <select
                className={
                  "cf-select cf-w-auto" +
                  (dirtyFields["HEVC encoding"] ? " cf-changed" : "")
                }
                value={hevcMode}
                onChange={(e) => setHevcMode(e.target.value as HevcMode)}
              >
                {HEVC_MODES.map((m) => (
                  <option key={m.value} value={m.value}>
                    {m.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Burn-in ASS/SSA subtitles</div>
              <div className="cf-row-help">
                Bake text subtitles (SRT / ASS / SSA) into the video
                instead of overlaying them as a selectable track. Costs
                CPU and forces a re-encode, but guarantees styled subs
                render identically on every client. Picture subs
                (PGS / VobSub) always burn regardless.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={burnAssSubtitles}
                aria-label="Burn-in ASS/SSA subtitles"
                className={"cf-switch" + (burnAssSubtitles ? " cf-on" : "")}
                onClick={() => setBurnAssSubtitles((v) => !v)}
              />
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">
                Two-pass loudness normalization
              </div>
              <div className="cf-row-help">
                Use precise per-file EBU R 128 measurements (from the
                loudness-analysis task) for measure-then-apply volume
                leveling, instead of the cheaper single-pass estimate.
                Only takes effect when normalization is on.
              </div>
            </div>
            <div className="cf-row-control">
              <button
                type="button"
                role="switch"
                aria-checked={twoPassLoudnorm}
                aria-label="Two-pass loudness normalization"
                className={"cf-switch" + (twoPassLoudnorm ? " cf-on" : "")}
                onClick={() => setTwoPassLoudnorm((v) => !v)}
              />
            </div>
          </div>

          <div className="cf-row">
            <div className="cf-row-main">
              <div className="cf-row-label">Transcode cache</div>
              <div className="cf-row-help">
                Where HLS segments are written. Set via the{" "}
                <span className="cf-mono">TRANSCODER_CACHE_DIR</span> env;
                read-only.
              </div>
            </div>
            <div className="cf-row-control cf-mono cf-faint">{cacheRoot}</div>
          </div>
        </div>
      </div>

      {/* ── Sticky save bar ───────────────────────────────────────── */}
      {dirtyCount > 0 && (
        <div className="cf-savebar">
          <div className="cf-sb-status">
            <span className="cf-dot" style={{ background: "var(--warn)" }} />
            <b>
              {dirtyCount} unsaved {dirtyCount === 1 ? "change" : "changes"}
            </b>
            {" · "}
            {dirtyLabels.slice(0, 3).join(", ")}
            {dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : ""}
          </div>
          <div className="cf-sb-actions">
            <button
              type="button"
              className="cf-btn cf-ghost cf-sm"
              onClick={discardChanges}
              disabled={saving}
            >
              Discard
            </button>
            <button
              type="button"
              className="cf-btn cf-primary cf-sm"
              onClick={saveSettings}
              disabled={saving}
            >
              {saving ? "Saving…" : "Save changes"}
            </button>
          </div>
        </div>
      )}

      {/* ── Active transcodes (no save state, WS-fed) ─────────────── */}
      <div className="cf-card" style={{ marginTop: 24 }}>
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Active transcodes</div>
            <div className="cf-sub">Live sessions in flight, pushed over the dashboard socket.</div>
          </div>
          {active.length > 0 && (
            <div className="cf-head-aside">
              <span className="cf-pill cf-ok">
                <span className="cf-dot" />
                {active.length} running
              </span>
            </div>
          )}
        </div>
        {active.length === 0 ? (
          <div className="cf-card-body cf-pad">
            <span className="cf-faint" style={{ fontSize: 13 }}>
              No transcodes in flight.
            </span>
          </div>
        ) : (
          <table className="cf-table">
            <thead>
              <tr>
                <th>Session</th>
                <th>User</th>
                <th>File</th>
                <th>Health</th>
                <th>Started</th>
                <th>Last seen</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {active.map((s) => (
                <tr key={s.id}>
                  <td className="cf-mono">{s.id}</td>
                  <td className="cf-muted">#{s.user_id}</td>
                  <td className="cf-muted">file #{s.media_file_id}</td>
                  <td>
                    <SessionHealthPills
                      transcode={s.transcode_health}
                      subtitle={s.subtitle_health ?? undefined}
                    />
                  </td>
                  <td className="cf-muted">
                    {formatRelative(nowMs - s.created_at)}
                  </td>
                  <td className="cf-muted">
                    {formatRelative(nowMs - s.last_seen_at)}
                  </td>
                  <td className="cf-num">
                    <button
                      type="button"
                      onClick={() => stopSession(s.id)}
                      className="cf-btn cf-ghost cf-tiny"
                    >
                      Stop
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

/// Renders one or two pills on the active-transcodes row representing
/// the ffmpeg-child health and the subtitle-extraction state. Healthy
/// + Ready (or absent subtitle) renders the neutral "live" pill so the
/// column never looks empty for a normal row.
function SessionHealthPills({
  transcode,
  subtitle,
}: {
  transcode: TranscodeHealth | undefined;
  subtitle: SubtitleHealth | undefined;
}) {
  const t = transcode ?? { kind: "healthy" as const };
  return (
    <div className="cf-flex cf-wrap cf-gap8">
      {t.kind === "exited" ? (
        <span className="cf-pill cf-err" title={t.detail}>
          <span className="cf-dot" />
          ffmpeg exited
        </span>
      ) : (
        <span className="cf-pill cf-ok">
          <span className="cf-dot" />
          live
        </span>
      )}
      {subtitle && subtitle.kind === "pending" && (
        <span className="cf-pill cf-info">
          <span className="cf-dot" />
          subs pending
        </span>
      )}
      {subtitle && subtitle.kind === "failed" && (
        <span className="cf-pill cf-warn" title={subtitle.reason}>
          <span className="cf-dot" />
          subs failed
        </span>
      )}
    </div>
  );
}

function formatRelative(ms: number): string {
  if (ms < 0 || ms < 1000) return "just now";
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return m > 0 ? `${h}h ${m}m ago` : `${h}h ago`;
}

function pickAutoLabel(encoders: string[]): string {
  if (encoders.includes("h264_nvenc")) return "NVIDIA NVENC";
  if (encoders.includes("h264_qsv")) return "Intel QuickSync";
  if (encoders.includes("h264_videotoolbox")) return "Apple VideoToolbox";
  if (encoders.includes("h264_vaapi")) return "VAAPI";
  if (encoders.includes("h264_amf")) return "AMD AMF";
  return "Software (libx264)";
}
