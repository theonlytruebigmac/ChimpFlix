"use client";

import { useEffect, useState } from "react";
import {
  admin as adminApi,
  type DashboardSession,
  type HevcMode,
  type ServerSettings,
  type TonemapAlgorithm,
  type TranscoderBackgroundPreset,
  type TranscoderCapabilities,
  type TranscoderEncoderPreset,
  type TranscoderHwAccel,
  type TranscoderHwStrictness,
  type TranscoderPreset,
} from "@/lib/chimpflix-api";

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
  /// Name of the encoder ffmpeg must have for this option to be
  /// usable. `null` for software (always works) and `auto` (picks
  /// whichever HW encoder is present, falls back to software).
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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
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
      .catch(() => {});

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

  const dirty =
    hwAccel !== baseline.transcoder_hw_accel ||
    maxConcurrent !== baseline.transcoder_max_concurrent ||
    maxCpuConcurrent !== baseline.transcoder_max_cpu_concurrent ||
    (ceiling === "" ? null : Number(ceiling)) !==
      baseline.transcoder_quality_ceiling_kbps ||
    encoderPreset !== baseline.transcoder_encoder_preset ||
    hwStrictness !== baseline.transcoder_hw_strictness ||
    backgroundPreset !== baseline.transcoder_background_preset ||
    maxBackgroundConcurrent !== baseline.transcoder_max_background_concurrent ||
    tonemapEnabled !== baseline.transcoder_hdr_tonemap_enabled ||
    tonemapAlgo !== baseline.transcoder_hdr_tonemap_algo ||
    hevcMode !== baseline.transcoder_hevc_encoding_mode ||
    gpuDevice !== baseline.transcoder_gpu_device;

  async function saveSettings() {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      const patch = {
        transcoder_hw_accel: hwAccel,
        transcoder_max_concurrent: maxConcurrent,
        transcoder_max_cpu_concurrent: maxCpuConcurrent,
        transcoder_quality_ceiling_kbps: ceiling === "" ? null : Number(ceiling),
        transcoder_encoder_preset: encoderPreset,
        transcoder_hw_strictness: hwStrictness,
        transcoder_background_preset: backgroundPreset,
        transcoder_max_background_concurrent: maxBackgroundConcurrent,
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
        transcoder_hdr_tonemap_enabled: tonemapEnabled,
        transcoder_hdr_tonemap_algo: tonemapAlgo,
        transcoder_hevc_encoding_mode: hevcMode,
        transcoder_gpu_device: gpuDevice,
      });
      setSaved(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function refreshPresets() {
    const r = await adminApi.transcoder.listPresets();
    setAllPresets(r.presets);
  }

  return (
    <div className="space-y-8">
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="mb-4 text-base font-semibold">Engine</h2>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
          <Field label="Hardware acceleration">
            <select
              value={hwAccel}
              onChange={(e) => setHwAccel(e.target.value as TranscoderHwAccel)}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            >
              {HW_ACCEL_OPTIONS.map((opt) => {
                // Each option requires a SPECIFIC encoder name
                // (`h264_nvenc`, `h264_qsv`, …) — the hwaccel decoder
                // list is a different thing and not what we care about
                // for encoding. Check against the encoder list so the
                // dropdown only enables options that will actually run.
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
            <p className="mt-1 text-xs text-white/50">
              ffmpeg {capabilities.ffmpeg_version ?? "?"} —{" "}
              {capabilities.h264_encoders.length === 0
                ? "no h264 encoders detected"
                : capabilities.h264_encoders.join(", ")}
            </p>
            {hwAccel === "auto" && (
              <p className="mt-1 text-xs text-white/50">
                Will pick:{" "}
                <span className="font-medium text-white/80">
                  {pickAutoLabel(capabilities.h264_encoders)}
                </span>
              </p>
            )}
          </Field>
          <Field
            label="Max concurrent transcodes"
            hint="Sessions exceeding this limit are rejected with 429."
          >
            <input
              type="number"
              min={1}
              max={64}
              value={maxConcurrent}
              onChange={(e) => setMaxConcurrent(Number(e.target.value))}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field
            label="Max concurrent CPU transcodes"
            hint="Sub-cap for software (libx264 / libx265) sessions only. A single CPU encode pegs N cores; capping these separately stops a wave of fallback-to-software encodes from starving GPU sessions. Default 1."
          >
            <input
              type="number"
              min={1}
              max={16}
              value={maxCpuConcurrent}
              onChange={(e) => setMaxCpuConcurrent(Number(e.target.value))}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field
            label="Quality ceiling (kbps)"
            hint="Blank = no cap. Sessions never exceed this bitrate."
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
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
          <Field
            label="Encoder preset"
            hint="Speed–quality dial: speed shaves CPU, quality spends more cycles for finer detail. Applied to whichever encoder is active above."
          >
            <select
              value={encoderPreset}
              onChange={(e) =>
                setEncoderPreset(e.target.value as TranscoderEncoderPreset)
              }
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            >
              <option value="speed">Speed (lowest CPU)</option>
              <option value="balanced">Balanced (default)</option>
              <option value="quality">Quality (slower)</option>
            </select>
          </Field>
          <Field
            label="Hardware strictness"
            hint="How aggressively to enforce HW use. Require HW refuses sessions that need software fallback for any stage (decode / filter / encode)."
          >
            <select
              value={hwStrictness}
              onChange={(e) =>
                setHwStrictness(e.target.value as TranscoderHwStrictness)
              }
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            >
              <option value="auto">Auto (HW where possible, SW fallback)</option>
              <option value="prefer_hw">Prefer HW (warn on fallback)</option>
              <option value="require_hw">Require HW (refuse fallback)</option>
            </select>
          </Field>
        </div>
        <div className="mt-4 flex items-center gap-3">
          <button
            disabled={!dirty || busy}
            onClick={saveSettings}
            className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
          >
            {busy ? "Saving…" : "Save engine settings"}
          </button>
          {saved && !dirty && (
            <span className="text-xs text-white/50">Saved.</span>
          )}
        </div>
        <details className="mt-4 text-xs text-white/50">
          <summary className="cursor-pointer hover:text-white/70">
            Capability detail (probed at startup)
          </summary>
          <div className="mt-2 grid grid-cols-1 gap-3 md:grid-cols-2">
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
            Decoder support is probed at server start by running a
            one-frame test through each hwaccel — so this reflects
            what your actual card can do (NVDEC AV1 needs Ampere+,
            AV1 in VAAPI needs RDNA2+), not the codec list the
            ffmpeg build was compiled with.
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
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="mb-1 text-base font-semibold">Background transcoding</h2>
        <p className="mb-4 text-xs text-white/55">
          The <code className="font-mono">optimize_versions</code> scheduled
          task pre-encodes media into operator-defined presets so weak
          clients don&apos;t need a live transcode. These dials trade CPU
          time / output size and protect live playback from background
          starvation. Always uses libx264 (no GPU).
        </p>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Field
            label="x264 preset"
            hint="Slower presets produce smaller files at the same quality, but consume more CPU per encode."
          >
            <select
              value={backgroundPreset}
              onChange={(e) =>
                setBackgroundPreset(
                  e.target.value as TranscoderBackgroundPreset,
                )
              }
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            >
              {BACKGROUND_PRESETS.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </Field>
          <Field
            label="Max concurrent background jobs"
            hint="Hard cap on how many optimize_versions jobs run per scheduler tick (every 30s)."
          >
            <input
              type="number"
              min={1}
              max={16}
              value={maxBackgroundConcurrent}
              onChange={(e) =>
                setMaxBackgroundConcurrent(Number(e.target.value))
              }
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            />
          </Field>
        </div>
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="mb-1 text-base font-semibold">HDR tone mapping</h2>
        <p className="mb-4 text-xs text-white/55">
          When the source is HDR (HDR10 / HLG / Dolby Vision) and the
          session is being re-encoded, ffmpeg applies a tonemap filter
          so the SDR output isn&apos;t washed out. Disabling skips the
          filter — saves CPU but the picture will look flat on SDR
          displays.
        </p>
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Field label="Tone map HDR sources">
            <label className="flex items-center gap-2 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm">
              <input
                type="checkbox"
                checked={tonemapEnabled}
                onChange={(e) => setTonemapEnabled(e.target.checked)}
              />
              <span>
                Apply HDR → SDR tonemap during reencode (recommended)
              </span>
            </label>
          </Field>
          <Field
            label="Tonemap algorithm"
            hint={
              TONEMAP_ALGOS.find((a) => a.value === tonemapAlgo)?.hint ?? ""
            }
          >
            <select
              value={tonemapAlgo}
              disabled={!tonemapEnabled}
              onChange={(e) => setTonemapAlgo(e.target.value as TonemapAlgorithm)}
              className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30 disabled:opacity-40"
            >
              {TONEMAP_ALGOS.map((a) => (
                <option key={a.value} value={a.value}>
                  {a.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="mb-1 text-base font-semibold">GPU device</h2>
        <p className="mb-4 text-xs text-white/55">
          Multi-GPU hosts can pin transcoding to a specific card.
          Auto lets the driver pick — fine for single-GPU systems
          (~99% of installs). Bad combinations (e.g. picking a VAAPI
          render node while NVENC is active) silently fall back to
          driver default at session-spawn time.
        </p>
        <Field
          label="Transcode device"
          hint={
            capabilities.gpu_devices.length === 0
              ? "No multi-GPU devices detected; the dropdown only shows Auto."
              : "Pinned to the chosen card for every new session. Restart not required."
          }
        >
          <select
            value={gpuDevice}
            onChange={(e) => setGpuDevice(e.target.value)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          >
            <option value="auto">Auto (driver picks)</option>
            {capabilities.gpu_devices.map((d) => (
              <option key={d.value} value={d.value}>
                {d.name}
              </option>
            ))}
          </select>
        </Field>
      </section>

      <section className="rounded-lg border border-white/10 bg-white/2 p-6">
        <h2 className="mb-1 text-base font-semibold">HEVC output</h2>
        <p className="mb-4 text-xs text-white/55">
          HEVC (H.265) produces ~30% smaller files at the same visual
          quality but isn&apos;t universally browser-supported. Output
          container is forced to fMP4 when HEVC is selected; the ABR
          fallback variant is disabled (HEVC ABR is a future expansion).
        </p>
        <Field
          label="HEVC encoding mode"
          hint={HEVC_MODES.find((m) => m.value === hevcMode)?.hint ?? ""}
        >
          <select
            value={hevcMode}
            onChange={(e) => setHevcMode(e.target.value as HevcMode)}
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          >
            {HEVC_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {m.label}
              </option>
            ))}
          </select>
        </Field>
      </section>

      <section>
        <h2 className="mb-3 flex items-center gap-2 text-base font-semibold">
          Active transcodes
          {active.length > 0 && (
            <span className="flex items-center gap-1 rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-emerald-300">
              <span className="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />
              {active.length} running
            </span>
          )}
        </h2>
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

      <section>
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-base font-semibold">Quality presets</h2>
          <button
            onClick={() => setShowAdd((v) => !v)}
            className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:px-3 sm:py-1.5 text-white hover:bg-red-600"
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
    if (!window.confirm(`Delete preset "${preset.name}"?`)) return;
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
          onClick={remove}
          className="rounded border border-white/15 px-2 py-1 text-xs text-white/50 hover:border-red-500/50 hover:text-red-300"
        >
          Delete
        </button>
      </td>
    </tr>
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
      <Field label="Name">
        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Mobile 360p"
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
        />
      </Field>
      <Field label="Max height (px)">
        <input
          type="number"
          value={maxHeight}
          min={0}
          max={4320}
          onChange={(e) => setMaxHeight(Number(e.target.value))}
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
        />
      </Field>
      <Field label="Max bitrate (kbps)">
        <input
          type="number"
          value={maxBitrate}
          min={0}
          max={200_000}
          onChange={(e) => setMaxBitrate(Number(e.target.value))}
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
        />
      </Field>
      <div className="flex items-end">
        <button
          disabled={busy || !name.trim()}
          onClick={submit}
          className="w-full rounded-md bg-red-500 px-3 py-2 text-sm font-semibold text-white hover:bg-red-600 disabled:opacity-50"
        >
          {busy ? "Creating…" : "Create"}
        </button>
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1 block text-sm font-medium">{label}</label>
      {children}
      {hint && <p className="mt-1 text-xs text-white/50">{hint}</p>}
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

/// One row of the "decoders probed at startup" detail. Empty list
/// means either the hwaccel isn't present or every codec probe
/// failed (no driver, no card, missing libavcodec encoder for the
/// probe's test stream). Either way the user gets a clear "none"
/// signal.
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

/// Mirror of HwAccel::auto_pick in the Rust transcoder — when the
/// operator sets the dropdown to "Auto", show which concrete encoder
/// will actually run. Keep priorities in sync if the backend's
/// `auto_pick` changes.
function pickAutoLabel(encoders: string[]): string {
  if (encoders.includes("h264_nvenc")) return "NVIDIA NVENC";
  if (encoders.includes("h264_qsv")) return "Intel QuickSync";
  if (encoders.includes("h264_videotoolbox")) return "Apple VideoToolbox";
  if (encoders.includes("h264_vaapi")) return "VAAPI";
  if (encoders.includes("h264_amf")) return "AMD AMF";
  return "Software (libx264)";
}
