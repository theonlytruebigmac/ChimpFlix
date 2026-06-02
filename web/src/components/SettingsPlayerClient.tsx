"use client";

import { useCallback, useState } from "react";

import { auth as authApi } from "@/lib/chimpflix-api";
import { usePrefs } from "@/lib/prefs";
import {
  DEFAULT_SUBTITLE_STYLE,
  cssFontFamilyForSubtitleStyle,
  type SubtitleEdge,
  type SubtitleFontFamily,
  type SubtitleStyle,
} from "@/lib/subtitle-style";

// Player prefs split: device-local (autoplay / trailer-mute / loudness /
// auto-skip) lives in localStorage via usePrefs; subtitle styling is
// server-synced per account via /api/v1/auth/me (phase 89). Rendered in
// the console design language (cf-* classes). The subtitle panel here is
// settings-local on purpose: the player's own SubtitleStylePanel lives
// outside `.cf-console`, so it can't share these scoped classes — both
// drive the same SubtitleStyle model + PATCH, so behavior matches.
export function SettingsPlayerClient({
  initialSubtitleStyle,
}: {
  initialSubtitleStyle: SubtitleStyle;
}) {
  const [prefs, updatePrefs] = usePrefs();
  const [subtitleStyle, setSubtitleStyleLocal] =
    useState<SubtitleStyle>(initialSubtitleStyle);
  const [saveError, setSaveError] = useState<string | null>(null);

  const setSubtitleStyle = useCallback((next: SubtitleStyle) => {
    setSubtitleStyleLocal(next);
    setSaveError(null);
    authApi
      .updateMe({
        subtitle_font_size_px: next.fontSizePx,
        subtitle_text_color: next.textColor,
        subtitle_background_color: next.backgroundColor,
        subtitle_font_family: next.fontFamily,
        subtitle_edge: next.edge,
        subtitle_bottom_inset_pct: next.bottomInsetPct,
      })
      .catch((e: unknown) => {
        // Keep the local pick visible so the viewer sees their choice,
        // but flag that it won't survive a refresh — a silent swallow
        // leaves users confused when settings revert.
        setSaveError(e instanceof Error ? e.message : String(e));
      });
  }, []);

  const patch = (p: Partial<SubtitleStyle>) =>
    setSubtitleStyle({ ...subtitleStyle, ...p });

  function resetEverything() {
    setSubtitleStyle(DEFAULT_SUBTITLE_STYLE);
    updatePrefs({
      autoplayNext: true,
      trailerMuted: true,
      audioNormalize: false,
      autoSkipIntro: true,
    });
  }

  return (
    <div>
      {/* ── playback behaviour (device-local) ───────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Playback behaviour</div>
            <div className="cf-sub">
              Saved on this device only — switch browsers and these reset to
              defaults.
            </div>
          </div>
          <div className="cf-head-aside">
            <span className="cf-pill">
              <svg
                style={{ width: 13, height: 13 }}
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
              >
                <rect x="3" y="4" width="18" height="12" rx="2" />
                <path d="M8 20h8M12 16v4" />
              </svg>
              This device
            </span>
          </div>
        </div>
        <div className="cf-card-body">
          <ToggleRow
            label="Autoplay next episode"
            help="Roll into the next episode of a series automatically."
            checked={prefs.autoplayNext}
            onChange={(v) => updatePrefs({ autoplayNext: v })}
          />
          <ToggleRow
            label="Mute hero trailers"
            help="Preview clips on the home page start without sound."
            checked={prefs.trailerMuted}
            onChange={(v) => updatePrefs({ trailerMuted: v })}
          />
          <ToggleRow
            label="Normalize audio loudness"
            help="Apply EBU R128 so quiet dialogue and loud action stay closer in volume. Disables the audio-copy fast path."
            checked={prefs.audioNormalize}
            onChange={(v) => updatePrefs({ audioNormalize: v })}
          />
          <ToggleRow
            label="Auto-skip intros"
            help="Jump past the title sequence when an intro marker plays. Credits still need a manual skip so you don't miss post-credit scenes."
            checked={prefs.autoSkipIntro}
            onChange={(v) => updatePrefs({ autoSkipIntro: v })}
          />
        </div>
      </div>

      {/* ── subtitle styling (synced) ───────────────────────────────── */}
      <div className="cf-card">
        <div className="cf-card-head">
          <div>
            <div className="cf-ttl">Subtitle styling</div>
            <div className="cf-sub">
              Applies everywhere you watch — browser-rendered and burned-in
              subtitles alike.
            </div>
          </div>
          <div className="cf-head-aside">
            <span className="cf-pill cf-info">
              <svg
                style={{ width: 13, height: 13 }}
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
              >
                <path d="M4 10a8 8 0 0 1 14-4l2 2M20 14a8 8 0 0 1-14 4l-2-2" />
                <path d="M18 4v4h-4M6 20v-4h4" />
              </svg>
              Synced to your account
            </span>
          </div>
        </div>
        <div className="cf-card-body cf-pad">
          <SubtitlePreview style={subtitleStyle} />

          <div style={{ marginTop: 8 }}>
            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-label">Size</div>
              </div>
              <div className="cf-row-control">
                <Seg
                  options={SIZE_OPTIONS.map((o) => ({
                    key: String(o.px),
                    label: o.label,
                  }))}
                  active={String(subtitleStyle.fontSizePx)}
                  onSelect={(k) => patch({ fontSizePx: Number(k) })}
                />
              </div>
            </div>

            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-label">Colour</div>
              </div>
              <div className="cf-row-control">
                <div className="cf-swatches">
                  {COLORS.map((c) => (
                    <button
                      key={c}
                      type="button"
                      aria-label={`Subtitle colour ${c}`}
                      aria-pressed={subtitleStyle.textColor === c}
                      className={`cf-swatch${
                        subtitleStyle.textColor === c ? " cf-on" : ""
                      }`}
                      style={{ background: c }}
                      onClick={() => patch({ textColor: c })}
                    />
                  ))}
                </div>
              </div>
            </div>

            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-label">Background</div>
              </div>
              <div className="cf-row-control">
                <Seg
                  options={BG_OPTIONS.map((o) => ({ key: o.key, label: o.label }))}
                  active={bgKey(subtitleStyle.backgroundColor)}
                  onSelect={(k) =>
                    patch({
                      backgroundColor:
                        BG_OPTIONS.find((o) => o.key === k)?.value ??
                        DEFAULT_SUBTITLE_STYLE.backgroundColor,
                    })
                  }
                />
              </div>
            </div>

            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-label">Edge</div>
              </div>
              <div className="cf-row-control">
                <Seg
                  options={EDGE_OPTIONS}
                  active={subtitleStyle.edge}
                  onSelect={(k) => patch({ edge: k as SubtitleEdge })}
                />
              </div>
            </div>

            <div className="cf-row">
              <div className="cf-row-main">
                <div className="cf-row-label">Font</div>
              </div>
              <div className="cf-row-control">
                <Seg
                  options={FONT_OPTIONS}
                  active={subtitleStyle.fontFamily}
                  onSelect={(k) => patch({ fontFamily: k as SubtitleFontFamily })}
                />
              </div>
            </div>

            <div className="cf-row cf-col">
              <div
                className="cf-flex cf-between"
                style={{ width: "100%" }}
              >
                <div className="cf-row-label">
                  Position{" "}
                  <span className="cf-faint" style={{ fontWeight: 400 }}>
                    — {subtitleStyle.bottomInsetPct}% from bottom
                  </span>
                </div>
                <button
                  type="button"
                  className="cf-btn cf-ghost cf-tiny"
                  onClick={() => setSubtitleStyle(DEFAULT_SUBTITLE_STYLE)}
                >
                  Reset to defaults
                </button>
              </div>
              <div className="cf-row-control">
                <input
                  className="cf-range"
                  type="range"
                  min={0}
                  max={45}
                  value={subtitleStyle.bottomInsetPct}
                  onChange={(e) =>
                    patch({ bottomInsetPct: Number(e.target.value) })
                  }
                />
              </div>
              <div
                className="cf-flex cf-between"
                style={{ width: "100%", fontSize: 11, color: "var(--faint)" }}
              >
                <span>Video edge</span>
                <span>Middle</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* ── save status ─────────────────────────────────────────────── */}
      <div className="cf-savebar">
        <div className="cf-sb-status">
          {saveError ? (
            <>
              <span className="cf-dot" style={{ background: "var(--err)" }} />
              Couldn&apos;t save subtitle style — {saveError}. Changes revert on
              refresh.
            </>
          ) : (
            <>
              <span className="cf-dot" style={{ background: "var(--ok)" }} />
              All changes saved
            </>
          )}
        </div>
        <div className="cf-sb-actions">
          <button
            type="button"
            className="cf-btn cf-ghost cf-sm"
            onClick={resetEverything}
          >
            Reset everything
          </button>
        </div>
      </div>
    </div>
  );
}

const SIZE_OPTIONS: { px: number; label: string }[] = [
  { px: 16, label: "Very small" },
  { px: 20, label: "Small" },
  { px: 24, label: "Medium" },
  { px: 30, label: "Large" },
  { px: 38, label: "Very large" },
];

const COLORS = ["#ffffff", "#f7e425", "#5ad65a", "#5adada", "#5a9bff", "#ff5ad6"];

const BG_OPTIONS: { key: string; label: string; value: string }[] = [
  { key: "none", label: "None", value: "transparent" },
  { key: "translucent", label: "Translucent", value: "rgba(0,0,0,0.55)" },
  { key: "solid", label: "Solid black", value: "rgba(0,0,0,1)" },
];

const EDGE_OPTIONS = [
  { key: "outline", label: "Outline" },
  { key: "shadow", label: "Shadow" },
  { key: "none", label: "None" },
];

const FONT_OPTIONS = [
  { key: "default", label: "Default" },
  { key: "sans", label: "Sans" },
  { key: "serif", label: "Serif" },
  { key: "mono", label: "Mono" },
];

/// Map a stored backgroundColor back to the segmented control's key.
function bgKey(bg: string): string {
  const t = bg.trim();
  if (t === "transparent") return "none";
  const m = t.match(/^rgba?\([^)]*,\s*([\d.]+)\s*\)$/);
  const alpha = m ? parseFloat(m[1]) : 1;
  if (alpha === 0) return "none";
  if (alpha >= 1) return "solid";
  return "translucent";
}

/// Live preview matching the mockup: a faux video frame with a single
/// caption line styled from the current SubtitleStyle.
function SubtitlePreview({ style }: { style: SubtitleStyle }) {
  const textShadow =
    style.edge === "outline"
      ? "0 0 4px #000,0 0 4px #000,0 0 4px #000"
      : style.edge === "shadow"
        ? "2px 2px 4px rgba(0,0,0,.9)"
        : "none";
  return (
    <div
      style={{
        position: "relative",
        height: 200,
        borderRadius: "var(--r-lg)",
        overflow: "hidden",
        display: "flex",
        alignItems: "flex-end",
        justifyContent: "center",
        background: "linear-gradient(135deg,#1d2b4d,#3b1d4d 55%,#4d1d2b)",
      }}
    >
      <div
        style={{
          position: "absolute",
          inset: 0,
          background:
            "radial-gradient(60% 80% at 50% 20%,rgba(255,255,255,.12),transparent)",
        }}
      />
      <div
        style={{
          marginBottom: 12 + style.bottomInsetPct * 2.4,
          padding: "4px 12px",
          borderRadius: 5,
          background: style.backgroundColor,
        }}
      >
        <span
          style={{
            fontSize: style.fontSizePx,
            fontWeight: 600,
            color: style.textColor,
            fontFamily:
              cssFontFamilyForSubtitleStyle(style.fontFamily) ?? undefined,
            textShadow,
          }}
        >
          You can&apos;t fight in here — this is the war room.
        </span>
      </div>
    </div>
  );
}

/// cf-* segmented control. Local to this file (the admin/ui Seg renders
/// Tailwind, not cf-* classes).
function Seg({
  options,
  active,
  onSelect,
}: {
  options: { key: string; label: string }[];
  active: string;
  onSelect: (key: string) => void;
}) {
  return (
    <div className="cf-seg">
      {options.map((o) => (
        <button
          key={o.key}
          type="button"
          className={o.key === active ? "cf-on" : ""}
          onClick={() => onSelect(o.key)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function ToggleRow({
  label,
  help,
  checked,
  onChange,
}: {
  label: string;
  help?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="cf-row">
      <div className="cf-row-main">
        <div className="cf-row-label">{label}</div>
        {help && <div className="cf-row-help">{help}</div>}
      </div>
      <div className="cf-row-control">
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          aria-label={label}
          className={`cf-switch${checked ? " cf-on" : ""}`}
          onClick={() => onChange(!checked)}
        />
      </div>
    </div>
  );
}
