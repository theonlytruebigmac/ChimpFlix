"use client";

import { useCallback, useState } from "react";

import { SubtitleStylePanel } from "@/components/ChimpFlixPlayer";
import { auth as authApi } from "@/lib/chimpflix-api";
import { usePrefs } from "@/lib/prefs";
import {
  type SubtitleStyle,
} from "@/lib/subtitle-style";

// Player prefs split: device-local (volume / autoplay / trailer-mute /
// loudness / auto-skip) lives in localStorage via usePrefs; subtitle
// styling is server-synced per account via /api/v1/auth/me (phase 89).
// The split mirrors what users actually want — "what does my account
// look like" follows the account, "what does this device do" follows
// the device.
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
        // Preview keeps the local pick so the viewer can see what they
        // chose, but flag that it won't survive a refresh — silent
        // swallow leaves users confused when settings revert.
        setSaveError(e instanceof Error ? e.message : String(e));
      });
  }, []);

  return (
    <div className="space-y-6">
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
      <p className="text-xs text-white/45">
        Toggles above are saved on this device only. Switch browsers and
        they reset to defaults.
      </p>

      <div>
        <h3 className="mb-1 text-xs font-semibold uppercase tracking-wider text-white/45">
          Subtitle styling
        </h3>
        <p className="mb-3 text-xs text-white/45">
          Saved to your account — follows you across devices. Applies to
          both browser-rendered subtitles and burned-in subtitles when
          transcoding text tracks.
        </p>
        <SubtitleStylePanel value={subtitleStyle} onChange={setSubtitleStyle} />
        {saveError && (
          <div
            role="alert"
            aria-live="assertive"
            className="mt-3 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300"
          >
            Couldn&apos;t save subtitle style: {saveError}. Changes will
            revert on next refresh.
          </div>
        )}
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
