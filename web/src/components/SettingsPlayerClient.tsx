"use client";

import { usePrefs } from "@/lib/prefs";

// Device-local player prefs. These live in localStorage rather than on the
// user record because they're per-browser ergonomics — what feels right on
// a TV-connected laptop differs from what feels right on a phone.
export function SettingsPlayerClient() {
  const [prefs, updatePrefs] = usePrefs();

  return (
    <div className="space-y-6">
      <p className="text-xs text-white/55">
        Saved on this device only. Switch browsers and these reset to defaults.
      </p>

      <Group>
        <ToggleRow
          label="Autoplay next episode"
          description="Roll into the next episode of a series automatically."
          checked={prefs.autoplayNext}
          onChange={(v) => updatePrefs({ autoplayNext: v })}
        />
        <ToggleRow
          label="Mute hero trailers"
          description="Preview clips on the home page start without sound."
          checked={prefs.trailerMuted}
          onChange={(v) => updatePrefs({ trailerMuted: v })}
        />
        <ToggleRow
          label="Normalize audio loudness"
          description="Apply EBU R128 normalization so quiet dialogue and loud action stay closer in volume. Disables the audio-copy fast path."
          checked={prefs.audioNormalize}
          onChange={(v) => updatePrefs({ audioNormalize: v })}
        />
        <ToggleRow
          label="Auto-skip intros"
          description="Jump past the title sequence automatically when an intro marker plays. Credits still require a manual skip so you don't miss post-credit scenes."
          checked={prefs.autoSkipIntro}
          onChange={(v) => updatePrefs({ autoSkipIntro: v })}
        />
      </Group>

      <div>
        <h3 className="mb-1 text-xs font-semibold uppercase tracking-wider text-white/45">
          Subtitle styling
        </h3>
        <p className="mb-3 text-xs text-white/45">
          Applies to external subtitles. Burned-in subtitles use these styles
          when transcoding text tracks.
        </p>
        <Group>
          <SliderRow
            label="Font size"
            value={prefs.subtitleFontScale}
            min={0.6}
            max={2.4}
            step={0.1}
            unit="×"
            onChange={(v) => updatePrefs({ subtitleFontScale: v })}
          />
          <ColorRow
            label="Text color"
            value={hexFromCss(prefs.subtitleColor) ?? "#ffffff"}
            onChange={(v) => updatePrefs({ subtitleColor: v })}
          />
          <SliderRow
            label="Background opacity"
            value={alphaFromRgba(prefs.subtitleBackground)}
            min={0}
            max={1}
            step={0.05}
            unit=""
            onChange={(v) =>
              updatePrefs({ subtitleBackground: `rgba(0, 0, 0, ${v})` })
            }
          />
          <SelectRow
            label="Position"
            value={prefs.subtitlePosition}
            options={[
              { value: "bottom", label: "Bottom (default)" },
              { value: "top", label: "Top" },
            ]}
            onChange={(v) =>
              updatePrefs({ subtitlePosition: v as "top" | "bottom" })
            }
          />
          <SelectRow
            label="Font family"
            value={prefs.subtitleFontFamily}
            options={[
              { value: "default", label: "Default" },
              { value: "sans", label: "Sans-serif" },
              { value: "serif", label: "Serif" },
              { value: "mono", label: "Monospace" },
            ]}
            onChange={(v) =>
              updatePrefs({
                subtitleFontFamily: v as
                  | "default"
                  | "sans"
                  | "serif"
                  | "mono",
              })
            }
          />
        </Group>
      </div>
    </div>
  );
}

/// Container for a stack of flat rows with hairline dividers — the
/// Account-style equivalent of a settings card without the rounded
/// surface. Each child row supplies its own padding via the *Row
/// helpers so the divider lands flush against the content.
function Group({ children }: { children: React.ReactNode }) {
  return (
    <div className="divide-y divide-white/5 border-y border-white/5">
      {children}
    </div>
  );
}

function ToggleRow({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start justify-between gap-4 py-3 text-sm transition-colors hover:bg-white/2">
      <span className="min-w-0">
        <span className="block text-white">{label}</span>
        {description && (
          <span className="mt-0.5 block text-xs text-white/50">
            {description}
          </span>
        )}
      </span>
      <span className="relative inline-flex h-6 w-11 shrink-0 items-center">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className="peer sr-only"
        />
        <span className="absolute inset-0 rounded-full bg-white/15 transition-colors peer-checked:bg-(--color-accent)" />
        <span className="absolute left-0.5 inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform peer-checked:translate-x-5" />
      </span>
    </label>
  );
}

function SliderRow({
  label,
  value,
  min,
  max,
  step,
  unit,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit: string;
  onChange: (v: number) => void;
}) {
  return (
    <label className="grid grid-cols-[1fr_auto] items-center gap-3 py-3 text-sm">
      <span className="text-white/70">{label}</span>
      <span className="flex items-center gap-3">
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(parseFloat(e.target.value))}
          className="w-40 accent-(--color-accent)"
        />
        <span className="w-12 text-right font-mono text-xs tabular-nums text-white/65">
          {value.toFixed(step < 1 ? 2 : 0)}
          {unit}
        </span>
      </span>
    </label>
  );
}

function ColorRow({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-3 py-3 text-sm">
      <span className="text-white/70">{label}</span>
      <input
        type="color"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-7 w-12 cursor-pointer rounded border border-white/10 bg-transparent"
      />
    </label>
  );
}

function SelectRow({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (v: string) => void;
}) {
  return (
    <label className="flex items-center justify-between gap-3 py-3 text-sm">
      <span className="text-white/70">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="rounded bg-white/10 px-3 py-1.5 text-xs outline-none ring-1 ring-white/10 focus:ring-(--color-accent)"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

/// Convert the most common stored color forms back to `#RRGGBB` so the
/// native color picker renders correctly. Returns null for shapes we
/// can't normalise (the input then falls back to white as the default).
function hexFromCss(css: string): string | null {
  const trimmed = css.trim();
  if (/^#[0-9a-fA-F]{6}$/.test(trimmed)) return trimmed.toLowerCase();
  return null;
}

/// Pull the alpha component out of an `rgba(...)` string; defaults to
/// 0.6 if the input doesn't parse so the slider always has a value.
function alphaFromRgba(css: string): number {
  const m = css.match(/rgba\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*,\s*([\d.]+)\s*\)/);
  if (!m) return 0.6;
  return parseFloat(m[1]);
}
