"use client";

import { useState } from "react";
import {
  admin as adminApi,
  type ServerSettings,
  type ServerSettingsUpdate,
} from "@/lib/chimpflix-api";

interface Props {
  settings: ServerSettings;
}

/// Consolidated Library settings — Plex's "Settings → Library" page
/// shape. One long flat form with grouped sections instead of the
/// previous per-page split (Phase 3 had Playback alone here; this
/// folds in the scan toggles, CW dials, completion threshold, and
/// database cache into a single surface).
///
/// All fields hit `/admin/settings` PATCH. Some take effect
/// immediately (CW dials, threshold), others need a server restart
/// (scan watcher, DB cache size); those flag a "Restart pending"
/// badge next to the input so the operator knows.
export function AdminLibrarySettingsClient({ settings }: Props) {
  const [scanAuto, setScanAuto] = useState(settings.scan_automatically);
  const [detectMarkersOnAdd, setDetectMarkersOnAdd] = useState(
    settings.detect_markers_on_add,
  );
  const [audioNormalize, setAudioNormalize] = useState(
    settings.audio_normalize_enabled,
  );
  const [scannerNice, setScannerNice] = useState(settings.scanner_nice_level);
  const [playedThreshold, setPlayedThreshold] = useState(
    settings.video_played_threshold_pct,
  );
  const [completionBehaviour, setCompletionBehaviour] = useState(
    settings.video_completion_behaviour,
  );
  const [cwMaxItems, setCwMaxItems] = useState(
    settings.continue_watching_max_items,
  );
  const [cwMaxAgeWeeks, setCwMaxAgeWeeks] = useState(
    settings.continue_watching_max_age_weeks,
  );
  const [cwIncludePremieres, setCwIncludePremieres] = useState(
    settings.continue_watching_include_premieres,
  );
  const [dbCacheMb, setDbCacheMb] = useState(settings.database_cache_size_mb);
  const [metadataLanguage, setMetadataLanguage] = useState(
    settings.metadata_language,
  );
  const [recentlyAddedDays, setRecentlyAddedDays] = useState(
    settings.recently_added_days,
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);

  const dirty =
    scanAuto !== settings.scan_automatically ||
    detectMarkersOnAdd !== settings.detect_markers_on_add ||
    audioNormalize !== settings.audio_normalize_enabled ||
    scannerNice !== settings.scanner_nice_level ||
    playedThreshold !== settings.video_played_threshold_pct ||
    completionBehaviour !== settings.video_completion_behaviour ||
    cwMaxItems !== settings.continue_watching_max_items ||
    cwMaxAgeWeeks !== settings.continue_watching_max_age_weeks ||
    cwIncludePremieres !== settings.continue_watching_include_premieres ||
    dbCacheMb !== settings.database_cache_size_mb ||
    metadataLanguage !== settings.metadata_language ||
    recentlyAddedDays !== settings.recently_added_days;

  // Restart-required hints. These are read once at server startup
  // (file_watcher::spawn, SqliteConnectOptions, TmdbClient::with_language);
  // we surface a badge so the operator isn't surprised when saving
  // doesn't immediately change behavior.
  const scanAutoChanged = scanAuto !== settings.scan_automatically;
  const dbCacheChanged = dbCacheMb !== settings.database_cache_size_mb;
  const niceChanged = scannerNice !== settings.scanner_nice_level;
  const metadataLanguageChanged =
    metadataLanguage !== settings.metadata_language;

  async function save() {
    setSaving(true);
    setError(null);
    try {
      const patch: ServerSettingsUpdate = {
        scan_automatically: scanAuto,
        detect_markers_on_add: detectMarkersOnAdd,
        audio_normalize_enabled: audioNormalize,
        scanner_nice_level: scannerNice,
        video_played_threshold_pct: playedThreshold,
        video_completion_behaviour: completionBehaviour,
        continue_watching_max_items: cwMaxItems,
        continue_watching_max_age_weeks: cwMaxAgeWeeks,
        continue_watching_include_premieres: cwIncludePremieres,
        database_cache_size_mb: dbCacheMb,
        metadata_language: metadataLanguage,
        recently_added_days: recentlyAddedDays,
      };
      await adminApi.settings.patch(patch);
      Object.assign(settings, patch);
      setSavedAt(Date.now());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (dirty && !saving) save();
      }}
      className="space-y-6"
    >
      {error && (
        <div className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      <Section
        title="Scanning"
        description="How the server keeps the library in sync with files on disk."
      >
        <Toggle
          label="Scan my library automatically"
          hint={
            scanAutoChanged
              ? "Server restart required for changes to take effect."
              : "When on, the filesystem watcher fires a library scan within seconds of a file appearing or disappearing. Manual scans and scheduled scan tasks always work, regardless of this toggle."
          }
          checked={scanAuto}
          onChange={setScanAuto}
          restartPending={scanAutoChanged}
        />
        <Toggle
          label="Detect intro / credits when media is added"
          hint="After each file-watcher scan, queue blackdetect on every new file lacking auto markers. Expensive (~30s/45-min episode) — leave off if your hardware is slow or your library churns a lot. Scheduled `detect_markers` task still runs regardless."
          checked={detectMarkersOnAdd}
          onChange={setDetectMarkersOnAdd}
        />
        <Field
          label="Run scanner ffmpeg at lower priority"
          hint={
            niceChanged
              ? "Server restart required for changes to take effect."
              : "Wraps background ffmpeg invocations (scanner probes, scheduled tasks: previews, chapter thumbs, loudness, markers) in `nice -n N`. 0 disables. 10–15 is a polite default that yields to live playback. Live transcode sessions always run at normal priority."
          }
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={19}
              value={scannerNice}
              onChange={(e) => setScannerNice(Number(e.target.value))}
              className="w-20 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
            />
            <span className="text-sm text-white/55">nice level</span>
            {niceChanged && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                Restart pending
              </span>
            )}
          </div>
        </Field>
      </Section>

      <Section
        title="Recently Added badge"
        description="How long a freshly-added item shows the red “Recently Added” ribbon on its Card."
      >
        <Field
          label="Window"
          hint="Set to 0 to disable the badge entirely. Default 14 days matches Netflix’s recency. Takes effect on the next config-poll — no rebuild needed."
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={365}
              value={recentlyAddedDays}
              onChange={(e) =>
                setRecentlyAddedDays(Number(e.target.value))
              }
              className="w-24 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
              required
            />
            <span className="text-sm text-white/55">
              {recentlyAddedDays === 0
                ? "badge disabled"
                : recentlyAddedDays === 1
                  ? "day"
                  : "days"}
            </span>
          </div>
        </Field>
      </Section>

      <Section
        title="Continue Watching"
        description="Caps on the in-progress rail and the threshold for marking items watched."
      >
        <Field label="Maximum items" hint="Hard cap on the rail. Plex default 40.">
          <input
            type="number"
            min={1}
            max={200}
            value={cwMaxItems}
            onChange={(e) => setCwMaxItems(Number(e.target.value))}
            className="w-32 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
            required
          />
        </Field>
        <Field
          label="Weeks to consider"
          hint="In-progress items last played more than this many weeks ago disappear from the rail. Set 0 to disable the time-window filter entirely."
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={520}
              value={cwMaxAgeWeeks}
              onChange={(e) => setCwMaxAgeWeeks(Number(e.target.value))}
              className="w-24 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
              required
            />
            <span className="text-sm text-white/55">weeks</span>
          </div>
        </Field>
        <Toggle
          label="Include season premieres in Continue Watching"
          hint="When a new season's first episode exists for a show you've watched, surface it on the Continue Watching rail. Off skips premieres entirely (rail shows only in-progress items)."
          checked={cwIncludePremieres}
          onChange={setCwIncludePremieres}
        />
        <Field
          label="Video played threshold"
          hint="The percentage at which the player auto-scrobbles a session as watched. Same number gates the Continue Watching upper bound so tiles disappear the moment we mark them played."
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={50}
              max={99}
              value={playedThreshold}
              onChange={(e) => setPlayedThreshold(Number(e.target.value))}
              className="w-24 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
              required
            />
            <span className="text-sm text-white/55">%</span>
          </div>
        </Field>
        <Field
          label="Video play completion"
          hint="What counts as 'watched'. `first_credits_marker` requires that the `detect_markers` task has found a credits marker for the file; falls back to the percentage threshold when it hasn't. `earliest_of_both` scrobbles at whichever lands first."
        >
          <select
            value={completionBehaviour}
            onChange={(e) =>
              setCompletionBehaviour(e.target.value as typeof completionBehaviour)
            }
            className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
          >
            <option value="threshold_pct">Percentage threshold only</option>
            <option value="first_credits_marker">
              First credits marker (fall back to threshold)
            </option>
            <option value="earliest_of_both">
              Earliest of either (recommended once markers are populated)
            </option>
          </select>
        </Field>
      </Section>

      <Section
        title="Metadata"
        description="Preferred language for TMDB-sourced descriptions, taglines, and localized titles."
      >
        <Field
          label="Metadata language"
          hint={
            metadataLanguageChanged
              ? "Server restart required for changes to take effect. Existing items keep their current text — run Refresh metadata on a library (or per-item) to re-pull in the new language."
              : "BCP-47 tag (e.g. en-US, ja-JP, de-DE). TMDB returns text in this language when available; for items missing a translation (common for niche anime) it falls back to the original language silently."
          }
        >
          <div className="flex items-center gap-2">
            <select
              value={METADATA_LANGUAGES.some((l) => l.tag === metadataLanguage) ? metadataLanguage : "__custom"}
              onChange={(e) => {
                if (e.target.value !== "__custom") {
                  setMetadataLanguage(e.target.value);
                }
              }}
              className="rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30"
            >
              {METADATA_LANGUAGES.map((l) => (
                <option key={l.tag} value={l.tag}>
                  {l.label} ({l.tag})
                </option>
              ))}
              <option value="__custom">Custom…</option>
            </select>
            <input
              type="text"
              value={metadataLanguage}
              onChange={(e) => setMetadataLanguage(e.target.value)}
              placeholder="en-US"
              maxLength={12}
              className="w-28 rounded-md border border-white/10 bg-black/30 px-3 py-2 font-mono text-sm outline-none focus:border-white/30"
            />
            {metadataLanguageChanged && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                Restart pending
              </span>
            )}
          </div>
        </Field>
      </Section>

      <Section
        title="Audio"
        description="EBU R 128 loudness normalization."
      >
        <Toggle
          label="Normalize loudness on every transcode session"
          hint="When on, ffmpeg's loudnorm filter is applied to every session by default — uses stored per-file measurements when the `analyze_loudness` task has run, otherwise generic targets. Users can still toggle off per-session via the player audio menu."
          checked={audioNormalize}
          onChange={setAudioNormalize}
        />
      </Section>

      <Section
        title="Database"
        description="SQLite tuning. Increase the page cache for libraries with tens of thousands of items."
      >
        <Field
          label="Page cache size"
          hint={
            dbCacheChanged
              ? "Server restart required for changes to take effect."
              : "0 = SQLite default (~2 MiB). 64 MiB is a sensible baseline for a busy library."
          }
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={4096}
              value={dbCacheMb}
              onChange={(e) => setDbCacheMb(Number(e.target.value))}
              className="w-24 rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm tabular-nums outline-none focus:border-white/30"
              required
            />
            <span className="text-sm text-white/55">MiB</span>
            {dbCacheChanged && (
              <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
                Restart pending
              </span>
            )}
          </div>
        </Field>
      </Section>

      <div className="flex items-center gap-3">
        <button
          type="submit"
          disabled={!dirty || saving}
          className="rounded-md bg-red-500 px-4 py-2.5 text-sm font-semibold sm:py-2 text-white transition-colors hover:bg-red-600 disabled:cursor-not-allowed disabled:bg-white/10 disabled:text-white/40"
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
        {savedAt && !dirty && !saving && (
          <span className="text-xs text-white/50">Saved.</span>
        )}
      </div>
    </form>
  );
}

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-lg border border-white/10 bg-white/2 p-6">
      <h2 className="text-base font-semibold">{title}</h2>
      {description && (
        <p className="mt-1 mb-4 max-w-2xl text-xs text-white/55">{description}</p>
      )}
      <div className="space-y-4">{children}</div>
    </section>
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

/// Plex-style checkbox toggle: label on the right, optional
/// "Restart pending" badge for restart-required settings.
function Toggle({
  label,
  hint,
  checked,
  onChange,
  restartPending,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  restartPending?: boolean;
}) {
  return (
    <div>
      <label className="flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          className="mt-1"
        />
        <div className="flex flex-wrap items-baseline gap-2">
          <span className="font-medium">{label}</span>
          {restartPending && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-amber-300">
              Restart pending
            </span>
          )}
        </div>
      </label>
      {hint && <p className="ml-7 mt-1 text-xs text-white/50">{hint}</p>}
    </div>
  );
}

// Curated dropdown of common TMDB-supported BCP-47 tags. Anything not
// in this list is reachable via the "Custom…" entry that exposes the
// freeform input — the server validates the tag shape, TMDB silently
// falls back to original language for unrecognised tags so a typo
// degrades gracefully rather than 500s.
const METADATA_LANGUAGES: ReadonlyArray<{ tag: string; label: string }> = [
  { tag: "en-US", label: "English (US)" },
  { tag: "en-GB", label: "English (UK)" },
  { tag: "ja-JP", label: "Japanese" },
  { tag: "ko-KR", label: "Korean" },
  { tag: "zh-CN", label: "Chinese (Simplified)" },
  { tag: "zh-TW", label: "Chinese (Traditional)" },
  { tag: "es-ES", label: "Spanish (Spain)" },
  { tag: "es-MX", label: "Spanish (Mexico)" },
  { tag: "fr-FR", label: "French" },
  { tag: "de-DE", label: "German" },
  { tag: "it-IT", label: "Italian" },
  { tag: "pt-BR", label: "Portuguese (Brazil)" },
  { tag: "pt-PT", label: "Portuguese (Portugal)" },
  { tag: "ru-RU", label: "Russian" },
  { tag: "nl-NL", label: "Dutch" },
  { tag: "sv-SE", label: "Swedish" },
  { tag: "pl-PL", label: "Polish" },
  { tag: "tr-TR", label: "Turkish" },
  { tag: "ar-SA", label: "Arabic" },
  { tag: "hi-IN", label: "Hindi" },
];
