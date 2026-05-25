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
  type TranscoderPreset,
} from "@/lib/chimpflix-api";
import { Pill, SaveBar, SettingsCard, SettingsRow } from "./ui";
import { ConfirmDialog } from "../ConfirmDialog";

const HEVC_MODES: ReadonlyArray<{ value: HevcMode; label: string; hint: string }> = [
  {
    value: "off",
    label: "Off (always H.264)",
    hint: "Default. Maximum browser compatibility — every client can decode H.264.",
  },
  {
    value: "when_client_supports",
    label: "When client supports HEVC",
    hint: "Safe-conservative: HEVC for Safari + some Edge / Chrome builds, H.264 otherwise.",
  },
  {
    value: "always",
    label: "Always (HEVC)",
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
  { value: "hable", label: "Hable", hint: "Filmic curve — default, balanced." },
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
  { value: "auto", label: "Auto (best available)", requires: null },
  { value: "none", label: "Software (libx264)", requires: null },
  { value: "nvenc", label: "NVENC (NVIDIA)", requires: "h264_nvenc" },
  { value: "qsv", label: "Quick Sync (Intel)", requires: "h264_qsv" },
  { value: "vaapi", label: "VAAPI (Linux Intel/AMD)", requires: "h264_vaapi" },
  { value: "videotoolbox", label: "VideoToolbox (macOS)", requires: "h264_videotoolbox" },
  { value: "amf", label: "AMF (AMD)", requires: "h264_amf" },
];

interface Props {
  capabilities: TranscoderCapabilities;
  cacheRoot: string;
  presets: TranscoderPreset[];
  settings: ServerSettings;
}

const INPUT_CLASS =
  "w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30";
const INPUT_CHANGED_CLASS =
  "w-full rounded-md border border-amber-400/40 bg-black/30 px-3 py-2 text-sm outline-none focus:border-amber-300";

export function AdminTranscoderClient({
  capabilities,
  cacheRoot,
  presets,
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
  const [hevcMode, setHevcMode] = useState<HevcMode>(
    baseline.transcoder_hevc_encoding_mode,
  );
  const [gpuDevice, setGpuDevice] = useState<string>(
    baseline.transcoder_gpu_device,
  );
  const [allPresets, setAllPresets] = useState(presets);
  const [showAdd, setShowAdd] = useState(false);
  const [error, setError] = useState<string | null>(null);
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

  // Per-field dirty flags. Drives the SettingsRow `changed` outline
  // and the SaveBar `dirtyCount` + summary.
  const dirtyFields: Record<string, boolean> = {
    "Hardware acceleration": hwAccel !== baseline.transcoder_hw_accel,
    "Max concurrent transcodes":
      maxConcurrent !== baseline.transcoder_max_concurrent,
    "Max CPU transcodes":
      maxCpuConcurrent !== baseline.transcoder_max_cpu_concurrent,
    "Quality ceiling":
      (ceiling === "" ? null : Number(ceiling)) !==
      baseline.transcoder_quality_ceiling_kbps,
    "Encoder preset": encoderPreset !== baseline.transcoder_encoder_preset,
    "Hardware strictness": hwStrictness !== baseline.transcoder_hw_strictness,
    "x264 preset":
      backgroundPreset !== baseline.transcoder_background_preset,
    "Max background jobs":
      maxBackgroundConcurrent !==
      baseline.transcoder_max_background_concurrent,
    "Job queue workers": jobWorkers !== baseline.job_workers,
    "Tone map HDR sources":
      tonemapEnabled !== baseline.transcoder_hdr_tonemap_enabled,
    "Tonemap algorithm":
      tonemapAlgo !== baseline.transcoder_hdr_tonemap_algo,
    "Transcode device": gpuDevice !== baseline.transcoder_gpu_device,
    "HEVC encoding mode":
      hevcMode !== baseline.transcoder_hevc_encoding_mode,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, isDirty]) => isDirty)
    .map(([label]) => label);
  const dirtyCount = dirtyLabels.length;

  async function saveSettings() {
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
      transcoder_hevc_encoding_mode: hevcMode,
      transcoder_gpu_device: gpuDevice,
    };
    await adminApi.settings.patch(patch);
    setBaseline({
      transcoder_hw_accel: hwAccel,
      transcoder_max_concurrent: maxConcurrent,
      transcoder_max_cpu_concurrent: maxCpuConcurrent,
      transcoder_quality_ceiling_kbps:
        ceiling === "" ? null : Number(ceiling),
      transcoder_encoder_preset: encoderPreset,
      transcoder_hw_strictness: hwStrictness,
      transcoder_background_preset: backgroundPreset,
      transcoder_max_background_concurrent: maxBackgroundConcurrent,
      job_workers: jobWorkers,
      transcoder_hdr_tonemap_enabled: tonemapEnabled,
      transcoder_hdr_tonemap_algo: tonemapAlgo,
      transcoder_hevc_encoding_mode: hevcMode,
      transcoder_gpu_device: gpuDevice,
    });
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
    setHevcMode(baseline.transcoder_hevc_encoding_mode);
    setGpuDevice(baseline.transcoder_gpu_device);
  }

  async function refreshPresets() {
    const r = await adminApi.transcoder.listPresets();
    setAllPresets(r.presets);
  }

  const tonemapHint =
    TONEMAP_ALGOS.find((a) => a.value === tonemapAlgo)?.hint ?? "";
  const hevcHint = HEVC_MODES.find((m) => m.value === hevcMode)?.hint ?? "";

  return (
    <div>
      {error && (
        <div className="mb-4 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {/* ── Engine ────────────────────────────────────────────────── */}
      <SettingsCard
        title="Engine"
        description={
          <>
            ffmpeg {capabilities.ffmpeg_version ?? "?"} —{" "}
            {capabilities.h264_encoders.length === 0
              ? "no h264 encoders detected"
              : capabilities.h264_encoders.join(", ")}
          </>
        }
      >
        <SettingsRow
          label="Hardware acceleration"
          help={
            hwAccel === "auto" ? (
              <>
                Auto will pick:{" "}
                <span className="font-medium text-white/80">
                  {pickAutoLabel(capabilities.h264_encoders)}
                </span>
              </>
            ) : (
              "Specific encoder used for live transcodes. Greys out options the host can't run."
            )
          }
          changed={dirtyFields["Hardware acceleration"]}
        >
          <select
            value={hwAccel}
            onChange={(e) => setHwAccel(e.target.value as TranscoderHwAccel)}
            className={
              dirtyFields["Hardware acceleration"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            {HW_ACCEL_OPTIONS.map((opt) => {
              const available =
                opt.requires == null ||
                capabilities.h264_encoders.includes(opt.requires);
              return (
                <option
                  key={opt.value}
                  value={opt.value}
                  disabled={!available && opt.value !== hwAccel}
                >
                  {opt.label}
                  {!available && " (not available)"}
                </option>
              );
            })}
          </select>
        </SettingsRow>

        <SettingsRow
          label="Max concurrent transcodes"
          help="Sessions exceeding this limit are rejected with 429."
          changed={dirtyFields["Max concurrent transcodes"]}
        >
          <input
            type="number"
            min={1}
            max={64}
            value={maxConcurrent}
            onChange={(e) => setMaxConcurrent(Number(e.target.value))}
            className={
              dirtyFields["Max concurrent transcodes"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          />
        </SettingsRow>

        <SettingsRow
          label="Max CPU transcodes"
          help="Sub-cap for software (libx264 / libx265) sessions only. A single CPU encode pegs N cores; capping these separately stops a wave of fallback-to-software encodes from starving GPU sessions. Default 1."
          changed={dirtyFields["Max CPU transcodes"]}
        >
          <input
            type="number"
            min={1}
            max={16}
            value={maxCpuConcurrent}
            onChange={(e) => setMaxCpuConcurrent(Number(e.target.value))}
            className={
              dirtyFields["Max CPU transcodes"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          />
        </SettingsRow>

        <SettingsRow
          label="Quality ceiling (kbps)"
          help="Blank = no cap. Sessions never exceed this bitrate."
          changed={dirtyFields["Quality ceiling"]}
        >
          <input
            type="number"
            min={100}
            max={200_000}
            value={ceiling}
            placeholder="Unlimited"
            onChange={(e) =>
              setCeiling(e.target.value === "" ? "" : Number(e.target.value))
            }
            className={
              dirtyFields["Quality ceiling"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          />
        </SettingsRow>

        <SettingsRow
          label="Encoder preset"
          help="Speed–quality dial: speed shaves CPU, quality spends more cycles for finer detail. Applied to whichever encoder is active above."
          changed={dirtyFields["Encoder preset"]}
        >
          <select
            value={encoderPreset}
            onChange={(e) =>
              setEncoderPreset(e.target.value as TranscoderEncoderPreset)
            }
            className={
              dirtyFields["Encoder preset"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            <option value="speed">Speed (lowest CPU)</option>
            <option value="balanced">Balanced (default)</option>
            <option value="quality">Quality (slower)</option>
          </select>
        </SettingsRow>

        <SettingsRow
          label="Hardware strictness"
          help="How aggressively to enforce HW use. Require HW refuses sessions that need software fallback for any stage (decode / filter / encode)."
          changed={dirtyFields["Hardware strictness"]}
        >
          <select
            value={hwStrictness}
            onChange={(e) =>
              setHwStrictness(e.target.value as TranscoderHwStrictness)
            }
            className={
              dirtyFields["Hardware strictness"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            <option value="auto">Auto (HW where possible, SW fallback)</option>
            <option value="prefer_hw">Prefer HW (warn on fallback)</option>
            <option value="require_hw">Require HW (refuse fallback)</option>
          </select>
        </SettingsRow>

        <details className="border-t border-white/10 px-5 py-3 text-xs text-white/50">
          <summary className="cursor-pointer hover:text-white/70">
            Capability detail (probed at startup)
          </summary>
          <div className="mt-3 grid grid-cols-1 gap-3 md:grid-cols-2">
            <div>
              <div className="text-white/40">H.264 hardware encoders</div>
              <div className="mt-0.5 font-mono">
                {capabilities.h264_encoders.length > 0
                  ? capabilities.h264_encoders.join(", ")
                  : "none"}
              </div>
            </div>
            <div>
              <div className="text-white/40">HEVC hardware encoders</div>
              <div className="mt-0.5 font-mono">
                {capabilities.hevc_encoders.length > 0
                  ? capabilities.hevc_encoders.join(", ")
                  : "none"}
              </div>
            </div>
            <DecoderRow label="NVDEC (CUDA) decoders" list={capabilities.decoders.cuda} />
            <DecoderRow label="VAAPI decoders" list={capabilities.decoders.vaapi} />
            <DecoderRow label="QSV decoders" list={capabilities.decoders.qsv} />
            <DecoderRow label="VideoToolbox decoders" list={capabilities.decoders.videotoolbox} />
          </div>
          <p className="mt-3 text-[11px] text-white/40">
            Decoder support is probed at server start by running a one-frame
            test through each hwaccel — so this reflects what your actual
            card can do, not the codec list ffmpeg was compiled with.
          </p>
          <div className="mt-3 flex flex-wrap items-baseline gap-2 text-[11px] text-white/40">
            <span className="text-white/55">Transcoder temp directory:</span>
            <code className="font-mono text-white/70">{cacheRoot}</code>
            <span className="text-white/40">
              (set via the <code className="font-mono">TRANSCODER_CACHE_DIR</code> env;
              requires server restart to change)
            </span>
          </div>
        </details>
      </SettingsCard>

      {/* ── Background transcoding ─────────────────────────────────── */}
      <SettingsCard
        title="Background transcoding"
        description={
          <>
            The <code className="font-mono">optimize_versions</code> scheduled
            task pre-encodes media into operator-defined presets so weak
            clients don&apos;t need a live transcode. These dials trade CPU
            time vs. output size and protect live playback from background
            starvation. Always uses libx264 (no GPU).
          </>
        }
      >
        <SettingsRow
          label="x264 preset"
          help="Slower presets produce smaller files at the same quality, but consume more CPU per encode."
          changed={dirtyFields["x264 preset"]}
        >
          <select
            value={backgroundPreset}
            onChange={(e) =>
              setBackgroundPreset(
                e.target.value as TranscoderBackgroundPreset,
              )
            }
            className={
              dirtyFields["x264 preset"] ? INPUT_CHANGED_CLASS : INPUT_CLASS
            }
          >
            {BACKGROUND_PRESETS.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </SettingsRow>
        <SettingsRow
          label="Max background jobs"
          help="Hard cap on how many optimize_versions jobs run per scheduler tick (every 30s)."
          changed={dirtyFields["Max background jobs"]}
        >
          <input
            type="number"
            min={1}
            max={16}
            value={maxBackgroundConcurrent}
            onChange={(e) =>
              setMaxBackgroundConcurrent(Number(e.target.value))
            }
            className={
              dirtyFields["Max background jobs"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          />
        </SettingsRow>
        <SettingsRow
          label="Job queue workers"
          help="How many worker tasks pull from the durable job queue (marker detection, loudness, subtitles, ratings). Each worker can run any kind, so raising this lets more pipeline kinds make progress in parallel when files pile up. Applies live — shrinking drains workers as soon as they finish their current job."
          changed={dirtyFields["Job queue workers"]}
        >
          <input
            type="number"
            min={1}
            max={16}
            value={jobWorkers}
            onChange={(e) => setJobWorkers(Number(e.target.value))}
            className={
              dirtyFields["Job queue workers"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          />
        </SettingsRow>
      </SettingsCard>

      {/* ── HDR tone mapping ──────────────────────────────────────── */}
      <SettingsCard
        title="HDR tone mapping"
        description="When the source is HDR (HDR10 / HLG / Dolby Vision) and the session is being re-encoded, ffmpeg applies a tonemap filter so the SDR output isn't washed out. Disabling skips the filter — saves CPU but the picture will look flat on SDR displays."
      >
        <SettingsRow
          label="Tone map HDR sources"
          help="Recommended unless you only stream to HDR-capable clients."
          changed={dirtyFields["Tone map HDR sources"]}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={tonemapEnabled}
              onChange={(e) => setTonemapEnabled(e.target.checked)}
            />
            <span>Apply HDR → SDR tonemap during reencode</span>
          </label>
        </SettingsRow>
        <SettingsRow
          label="Tonemap algorithm"
          help={tonemapHint}
          changed={dirtyFields["Tonemap algorithm"]}
        >
          <select
            value={tonemapAlgo}
            disabled={!tonemapEnabled}
            onChange={(e) => setTonemapAlgo(e.target.value as TonemapAlgorithm)}
            className={`${
              dirtyFields["Tonemap algorithm"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            } disabled:opacity-40`}
          >
            {TONEMAP_ALGOS.map((a) => (
              <option key={a.value} value={a.value}>
                {a.label}
              </option>
            ))}
          </select>
        </SettingsRow>
      </SettingsCard>

      {/* ── GPU device ────────────────────────────────────────────── */}
      <SettingsCard
        title="GPU device"
        description="Multi-GPU hosts can pin transcoding to a specific card. Auto lets the driver pick — fine for single-GPU systems (~99% of installs)."
      >
        <SettingsRow
          label="Transcode device"
          help={
            capabilities.gpu_devices.length === 0
              ? "No multi-GPU devices detected; the dropdown only shows Auto."
              : "Pinned to the chosen card for every new session. Restart not required."
          }
          changed={dirtyFields["Transcode device"]}
        >
          <select
            value={gpuDevice}
            onChange={(e) => setGpuDevice(e.target.value)}
            className={
              dirtyFields["Transcode device"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            <option value="auto">Auto (driver picks)</option>
            {capabilities.gpu_devices.map((d) => (
              <option key={d.value} value={d.value}>
                {d.name}
              </option>
            ))}
          </select>
        </SettingsRow>
      </SettingsCard>

      {/* ── HEVC ──────────────────────────────────────────────────── */}
      <SettingsCard
        title="HEVC output"
        description="HEVC (H.265) produces ~30% smaller files at the same visual quality but isn't universally browser-supported. Output container is forced to fMP4 when HEVC is selected; the ABR fallback variant is disabled (HEVC ABR is a future expansion)."
      >
        <SettingsRow
          label="HEVC encoding mode"
          help={hevcHint}
          changed={dirtyFields["HEVC encoding mode"]}
        >
          <select
            value={hevcMode}
            onChange={(e) => setHevcMode(e.target.value as HevcMode)}
            className={
              dirtyFields["HEVC encoding mode"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }
          >
            {HEVC_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </SettingsRow>
      </SettingsCard>

      {/* ── Sticky save bar ───────────────────────────────────────── */}
      <SaveBar
        dirtyCount={dirtyCount}
        summary={dirtyLabels.slice(0, 3).join(", ") +
          (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "")}
        onSave={saveSettings}
        onDiscard={discardChanges}
      />

      {/* ── Active transcodes (no save state) ─────────────────────── */}
      <section className="mt-6">
        <div className="mb-3 flex items-center gap-2">
          <h2 className="text-base font-semibold">Active transcodes</h2>
          {active.length > 0 && (
            <Pill tone="ok" dot>
              {active.length} running
            </Pill>
          )}
        </div>
        {active.length === 0 ? (
          <div className="rounded-lg border border-dashed border-white/15 bg-white/2 p-6 text-center text-sm text-white/50">
            No transcodes in flight.
          </div>
        ) : (
          <div className="overflow-hidden rounded-lg border border-white/10">
            <table className="w-full text-sm">
              <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
                <tr>
                  <th className="px-4 py-2">Session</th>
                  <th className="px-4 py-2">User</th>
                  <th className="px-4 py-2">File</th>
                  <th className="px-4 py-2">Health</th>
                  <th className="px-4 py-2">Started</th>
                  <th className="px-4 py-2">Last seen</th>
                  <th className="px-4 py-2" />
                </tr>
              </thead>
              <tbody>
                {active.map((s) => (
                  <tr key={s.id} className="border-t border-white/5">
                    <td className="whitespace-nowrap px-4 py-2 font-mono text-xs">
                      {s.id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      #{s.user_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/70">
                      file #{s.media_file_id}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2">
                      <SessionHealthPills
                        transcode={s.transcode_health}
                        subtitle={s.subtitle_health ?? undefined}
                      />
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatRelative(nowMs - s.created_at)}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-white/60">
                      {formatRelative(nowMs - s.last_seen_at)}
                    </td>
                    <td className="whitespace-nowrap px-4 py-2 text-right">
                      <button
                        onClick={() => stopSession(s.id)}
                        className="rounded border border-white/10 px-2 py-1 text-xs text-white/70 hover:bg-white/5"
                      >
                        Stop
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {/* ── Quality presets (CRUD) ────────────────────────────────── */}
      <section className="mt-6">
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-base font-semibold">Quality presets</h2>
          <button
            onClick={() => setShowAdd((v) => !v)}
            className="rounded-md bg-accent px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-accent-hover"
          >
            {showAdd ? "Cancel" : "+ New preset"}
          </button>
        </div>
        {showAdd && (
          <NewPresetForm
            onCreated={async () => {
              setShowAdd(false);
              await refreshPresets();
            }}
            onError={setError}
          />
        )}
        <div className="overflow-hidden rounded-lg border border-white/10">
          <table className="w-full text-sm">
            <thead className="bg-white/5 text-left text-xs uppercase tracking-wider text-white/40">
              <tr>
                <th className="px-4 py-2">Name</th>
                <th className="px-4 py-2">Max height</th>
                <th className="px-4 py-2">Max video kbps</th>
                <th className="px-4 py-2">Audio</th>
                <th className="px-4 py-2">Enabled</th>
                <th className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {allPresets.map((p) => (
                <PresetRow
                  key={p.id}
                  preset={p}
                  onChanged={refreshPresets}
                  onError={setError}
                />
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

function PresetRow({
  preset,
  onChanged,
  onError,
}: {
  preset: TranscoderPreset;
  onChanged: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [askDelete, setAskDelete] = useState(false);

  async function toggle() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.transcoder.updatePreset(preset.id, { enabled: !preset.enabled });
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setAskDelete(false);
    setBusy(true);
    onError(null);
    try {
      await adminApi.transcoder.deletePreset(preset.id);
      await onChanged();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
    <tr className="border-t border-white/5">
      <td className="px-4 py-2 font-medium">{preset.name}</td>
      <td className="px-4 py-2 tabular-nums text-white/70">
        {preset.max_height === 0 ? "—" : preset.max_height}
      </td>
      <td className="px-4 py-2 tabular-nums text-white/70">
        {preset.max_video_bitrate_kbps === 0 ? "—" : preset.max_video_bitrate_kbps}
      </td>
      <td className="px-4 py-2 text-white/70">
        {preset.audio_codec} @ {preset.audio_bitrate_kbps}k
      </td>
      <td className="px-4 py-2">
        <button
          disabled={busy}
          onClick={toggle}
          className={`rounded border px-2 py-0.5 text-[10px] uppercase tracking-wider ${preset.enabled ? "border-emerald-500/40 text-emerald-300" : "border-white/15 text-white/50"}`}
        >
          {preset.enabled ? "Enabled" : "Disabled"}
        </button>
      </td>
      <td className="whitespace-nowrap px-4 py-2 text-right">
        <button
          disabled={busy}
          onClick={() => setAskDelete(true)}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/50 hover:border-red-500/50 hover:text-red-300"
        >
          Delete
        </button>
      </td>
    </tr>
    {askDelete && (
      <ConfirmDialog
        title={`Delete preset "${preset.name}"?`}
        body="Active sessions using this preset keep running. New sessions will fall back to the default."
        confirmLabel="Delete"
        destructive
        busy={busy}
        onConfirm={() => void remove()}
        onCancel={() => setAskDelete(false)}
      />
    )}
    </>
  );
}

function NewPresetForm({
  onCreated,
  onError,
}: {
  onCreated: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [name, setName] = useState("");
  const [maxHeight, setMaxHeight] = useState(720);
  const [maxBitrate, setMaxBitrate] = useState(4000);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setBusy(true);
    onError(null);
    try {
      await adminApi.transcoder.createPreset({
        name: name.trim(),
        max_video_bitrate_kbps: maxBitrate,
        max_height: maxHeight,
      });
      setName("");
      await onCreated();
    } catch (e) {
      onError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mb-3 grid grid-cols-1 gap-3 rounded-lg border border-white/10 bg-white/2 p-4 md:grid-cols-4">
      <PresetField label="Name">
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Mobile 360p"
          className={INPUT_CLASS}
        />
      </PresetField>
      <PresetField label="Max height (px)">
        <input
          type="number"
          value={maxHeight}
          min={0}
          max={4320}
          onChange={(e) => setMaxHeight(Number(e.target.value))}
          className={INPUT_CLASS}
        />
      </PresetField>
      <PresetField label="Max bitrate (kbps)">
        <input
          type="number"
          value={maxBitrate}
          min={0}
          max={200_000}
          onChange={(e) => setMaxBitrate(Number(e.target.value))}
          className={INPUT_CLASS}
        />
      </PresetField>
      <div className="flex items-end">
        <button
          disabled={busy || !name.trim()}
          onClick={submit}
          className="w-full rounded-md bg-accent px-3 py-2 text-sm font-semibold text-white hover:bg-accent-hover disabled:opacity-50"
        >
          {busy ? "Creating…" : "Create"}
        </button>
      </div>
    </div>
  );
}

function PresetField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1 block text-sm font-medium">{label}</label>
      {children}
    </div>
  );
}

/// Renders one or two pills on the active-transcodes row representing
/// the ffmpeg-child health and the subtitle-extraction state. Healthy
/// + Ready (or absent subtitle) renders the neutral "Live" pill so the
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
    <div className="flex flex-wrap items-center gap-1.5">
      {t.kind === "exited" ? (
        <span title={t.detail}>
          <Pill tone="bad">ffmpeg exited</Pill>
        </span>
      ) : (
        <Pill tone="ok">live</Pill>
      )}
      {subtitle && subtitle.kind === "pending" && (
        <Pill tone="info">subs pending</Pill>
      )}
      {subtitle && subtitle.kind === "failed" && (
        <span title={subtitle.reason}>
          <Pill tone="warn">subs failed</Pill>
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

function DecoderRow({ label, list }: { label: string; list: string[] }) {
  return (
    <div>
      <div className="text-white/40">{label}</div>
      <div className="mt-0.5 font-mono">
        {list.length > 0 ? list.join(", ") : "none"}
      </div>
    </div>
  );
}

function pickAutoLabel(encoders: string[]): string {
  if (encoders.includes("h264_nvenc")) return "NVIDIA NVENC";
  if (encoders.includes("h264_qsv")) return "Intel QuickSync";
  if (encoders.includes("h264_videotoolbox")) return "Apple VideoToolbox";
  if (encoders.includes("h264_vaapi")) return "VAAPI";
  if (encoders.includes("h264_amf")) return "AMD AMF";
  return "Software (libx264)";
}
