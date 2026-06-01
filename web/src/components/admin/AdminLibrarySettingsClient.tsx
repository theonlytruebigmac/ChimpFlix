"use client";

import { useState } from "react";
import { Switch } from "@/components/admin/ui";
import {
  admin as adminApi,
  type ServerSettings,
  type ServerSettingsUpdate,
} from "@/lib/chimpflix-api";
import { ErrorBanner, Pill, SaveBar, SettingsCard, SettingsRow } from "./ui";

interface Props {
  settings: ServerSettings;
}

const INPUT_CLASS =
  "rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm outline-none focus:border-white/30";
const INPUT_CHANGED_CLASS =
  "rounded-md border border-amber-400/40 bg-black/30 px-3 py-2 text-sm outline-none focus:border-amber-300";

/// Periodic-scan interval options, mirroring Plex's "Library scan
/// interval" dropdown. `value` matches the scheduler frequency tokens the
/// backend understands (validated server-side in admin/settings::validate).
const SCAN_INTERVALS: { value: string; label: string }[] = [
  { value: "every_15_minutes", label: "every 15 minutes" },
  { value: "every_30_minutes", label: "every 30 minutes" },
  { value: "hourly", label: "hourly" },
  { value: "every_2_hours", label: "every 2 hours" },
  { value: "every_6_hours", label: "every 6 hours" },
  { value: "every_12_hours", label: "every 12 hours" },
  { value: "daily", label: "daily" },
];

/// Consolidated Library settings — Plex's "Settings → Library" page
/// shape. One stack of SettingsCards over fields that mostly hit
/// /admin/settings PATCH. Some take effect immediately (CW dials,
/// threshold), others need a server restart (scan watcher, DB cache);
/// those show a "Restart pending" pill next to the input.
export function AdminLibrarySettingsClient({ settings }: Props) {
  const [baseline, setBaseline] = useState({
    scan_automatically: settings.scan_automatically,
    file_watcher_use_polling: settings.file_watcher_use_polling,
    file_watcher_poll_interval_secs: settings.file_watcher_poll_interval_secs,
    periodic_scan_enabled: settings.periodic_scan_enabled,
    periodic_scan_frequency: settings.periodic_scan_frequency,
    empty_trash_after_scan: settings.empty_trash_after_scan,
    audio_normalize_enabled: settings.audio_normalize_enabled,
    subtitle_default_offset_ms: settings.subtitle_default_offset_ms,
    scanner_nice_level: settings.scanner_nice_level,
    video_played_threshold_pct: settings.video_played_threshold_pct,
    video_completion_behaviour: settings.video_completion_behaviour,
    continue_watching_max_items: settings.continue_watching_max_items,
    continue_watching_max_age_weeks: settings.continue_watching_max_age_weeks,
    continue_watching_include_premieres:
      settings.continue_watching_include_premieres,
    database_cache_size_mb: settings.database_cache_size_mb,
    metadata_language: settings.metadata_language,
    recently_added_days: settings.recently_added_days,
  });
  const [scanAuto, setScanAuto] = useState(baseline.scan_automatically);
  const [watcherPolling, setWatcherPolling] = useState(
    baseline.file_watcher_use_polling,
  );
  const [watcherPollSecs, setWatcherPollSecs] = useState(
    baseline.file_watcher_poll_interval_secs,
  );
  const [periodicScanEnabled, setPeriodicScanEnabled] = useState(
    baseline.periodic_scan_enabled,
  );
  const [periodicScanFrequency, setPeriodicScanFrequency] = useState(
    baseline.periodic_scan_frequency,
  );
  const [emptyTrashAfterScan, setEmptyTrashAfterScan] = useState(
    baseline.empty_trash_after_scan,
  );
  const [audioNormalize, setAudioNormalize] = useState(
    baseline.audio_normalize_enabled,
  );
  const [subtitleDefaultOffsetMs, setSubtitleDefaultOffsetMs] = useState(
    baseline.subtitle_default_offset_ms,
  );
  const [scannerNice, setScannerNice] = useState(baseline.scanner_nice_level);
  const [playedThreshold, setPlayedThreshold] = useState(
    baseline.video_played_threshold_pct,
  );
  const [completionBehaviour, setCompletionBehaviour] = useState(
    baseline.video_completion_behaviour,
  );
  const [cwMaxItems, setCwMaxItems] = useState(
    baseline.continue_watching_max_items,
  );
  const [cwMaxAgeWeeks, setCwMaxAgeWeeks] = useState(
    baseline.continue_watching_max_age_weeks,
  );
  const [cwIncludePremieres, setCwIncludePremieres] = useState(
    baseline.continue_watching_include_premieres,
  );
  const [dbCacheMb, setDbCacheMb] = useState(baseline.database_cache_size_mb);
  const [metadataLanguage, setMetadataLanguage] = useState(
    baseline.metadata_language,
  );
  const [recentlyAddedDays, setRecentlyAddedDays] = useState(
    baseline.recently_added_days,
  );
  const [error, setError] = useState<string | null>(null);

  const dirtyFields: Record<string, boolean> = {
    "Auto-scan": scanAuto !== baseline.scan_automatically,
    "Watcher polling":
      watcherPolling !== baseline.file_watcher_use_polling,
    "Watcher poll interval":
      watcherPollSecs !== baseline.file_watcher_poll_interval_secs,
    "Periodic scan": periodicScanEnabled !== baseline.periodic_scan_enabled,
    "Scan interval":
      periodicScanFrequency !== baseline.periodic_scan_frequency,
    "Empty trash after scan":
      emptyTrashAfterScan !== baseline.empty_trash_after_scan,
    "Audio normalize": audioNormalize !== baseline.audio_normalize_enabled,
    "Subtitle default offset":
      subtitleDefaultOffsetMs !== baseline.subtitle_default_offset_ms,
    "Scanner nice level": scannerNice !== baseline.scanner_nice_level,
    "Watched threshold":
      playedThreshold !== baseline.video_played_threshold_pct,
    "Completion behaviour":
      completionBehaviour !== baseline.video_completion_behaviour,
    "CW max items": cwMaxItems !== baseline.continue_watching_max_items,
    "CW age weeks":
      cwMaxAgeWeeks !== baseline.continue_watching_max_age_weeks,
    "CW include premieres":
      cwIncludePremieres !== baseline.continue_watching_include_premieres,
    "DB page cache": dbCacheMb !== baseline.database_cache_size_mb,
    "Metadata language": metadataLanguage !== baseline.metadata_language,
    "Recently-added window":
      recentlyAddedDays !== baseline.recently_added_days,
  };
  const dirtyLabels = Object.entries(dirtyFields)
    .filter(([, isDirty]) => isDirty)
    .map(([label]) => label);
  const dirtyCount = dirtyLabels.length;

  const scanAutoChanged = dirtyFields["Auto-scan"];
  const watcherPollingChanged = dirtyFields["Watcher polling"];
  const watcherPollSecsChanged = dirtyFields["Watcher poll interval"];
  const periodicScanChanged = dirtyFields["Periodic scan"];
  const scanIntervalChanged = dirtyFields["Scan interval"];
  const emptyTrashChanged = dirtyFields["Empty trash after scan"];
  const dbCacheChanged = dirtyFields["DB page cache"];
  const niceChanged = dirtyFields["Scanner nice level"];
  const metadataLanguageChanged = dirtyFields["Metadata language"];

  async function save() {
    setError(null);
    const patch: ServerSettingsUpdate = {
      scan_automatically: scanAuto,
      file_watcher_use_polling: watcherPolling,
      file_watcher_poll_interval_secs: watcherPollSecs,
      periodic_scan_enabled: periodicScanEnabled,
      periodic_scan_frequency: periodicScanFrequency,
      empty_trash_after_scan: emptyTrashAfterScan,
      audio_normalize_enabled: audioNormalize,
      subtitle_default_offset_ms: subtitleDefaultOffsetMs,
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
    setBaseline({
      scan_automatically: scanAuto,
      file_watcher_use_polling: watcherPolling,
      file_watcher_poll_interval_secs: watcherPollSecs,
      periodic_scan_enabled: periodicScanEnabled,
      periodic_scan_frequency: periodicScanFrequency,
      empty_trash_after_scan: emptyTrashAfterScan,
      audio_normalize_enabled: audioNormalize,
      subtitle_default_offset_ms: subtitleDefaultOffsetMs,
      scanner_nice_level: scannerNice,
      video_played_threshold_pct: playedThreshold,
      video_completion_behaviour: completionBehaviour,
      continue_watching_max_items: cwMaxItems,
      continue_watching_max_age_weeks: cwMaxAgeWeeks,
      continue_watching_include_premieres: cwIncludePremieres,
      database_cache_size_mb: dbCacheMb,
      metadata_language: metadataLanguage,
      recently_added_days: recentlyAddedDays,
    });
  }

  function discard() {
    setScanAuto(baseline.scan_automatically);
    setWatcherPolling(baseline.file_watcher_use_polling);
    setWatcherPollSecs(baseline.file_watcher_poll_interval_secs);
    setPeriodicScanEnabled(baseline.periodic_scan_enabled);
    setPeriodicScanFrequency(baseline.periodic_scan_frequency);
    setEmptyTrashAfterScan(baseline.empty_trash_after_scan);
    setAudioNormalize(baseline.audio_normalize_enabled);
    setSubtitleDefaultOffsetMs(baseline.subtitle_default_offset_ms);
    setScannerNice(baseline.scanner_nice_level);
    setPlayedThreshold(baseline.video_played_threshold_pct);
    setCompletionBehaviour(baseline.video_completion_behaviour);
    setCwMaxItems(baseline.continue_watching_max_items);
    setCwMaxAgeWeeks(baseline.continue_watching_max_age_weeks);
    setCwIncludePremieres(baseline.continue_watching_include_premieres);
    setDbCacheMb(baseline.database_cache_size_mb);
    setMetadataLanguage(baseline.metadata_language);
    setRecentlyAddedDays(baseline.recently_added_days);
  }

  return (
    <div>
      <ErrorBanner error={error} className="mb-4" />

      <SettingsCard
        title="Scanning"
        description="How the server keeps the library in sync with files on disk."
      >
        <SettingsRow
          label="Scan my library automatically"
          help={
            scanAutoChanged
              ? "Server restart required for changes to take effect."
              : "When on, the filesystem watcher fires a library scan within seconds of a file appearing or disappearing. Manual scans and scheduled scan tasks always work, regardless of this toggle."
          }
          changed={scanAutoChanged}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={scanAuto} onChange={setScanAuto} />
            <span>Watch the filesystem and scan on change</span>
            {scanAutoChanged && <Pill tone="warn">Restart pending</Pill>}
          </label>
        </SettingsRow>
        <SettingsRow
          label="Use polling watcher (NFS/SMB)"
          help={
            watcherPollingChanged
              ? "Server restart required for changes to take effect."
              : "Inotify (the default) does not see filesystem events on NFS/SMB mounts or bind-mounts that don't propagate inotify into the container. If you've been adding files and they only show up after a manual scan, turn this on. Costs more CPU + I/O than inotify."
          }
          changed={watcherPollingChanged}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={watcherPolling} onChange={setWatcherPolling} disabled={!scanAuto} />
            <span>Recursively stat watched roots on an interval</span>
            {watcherPollingChanged && <Pill tone="warn">Restart pending</Pill>}
          </label>
        </SettingsRow>
        <SettingsRow
          label="Polling interval"
          help={
            watcherPollSecsChanged
              ? "Server restart required for changes to take effect."
              : "How often the polling watcher rescans every watched root. Lower = faster detection but more I/O. Default 30s. Only consulted when polling is on."
          }
          changed={watcherPollSecsChanged}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={5}
              max={3600}
              value={watcherPollSecs}
              disabled={!watcherPolling}
              onChange={(e) => setWatcherPollSecs(Number(e.target.value))}
              className={`w-24 tabular-nums ${
                watcherPollSecsChanged ? INPUT_CHANGED_CLASS : INPUT_CLASS
              } disabled:opacity-40`}
            />
            <span className="text-sm text-white/55">seconds</span>
            {watcherPollSecsChanged && <Pill tone="warn">Restart pending</Pill>}
          </div>
        </SettingsRow>
        <SettingsRow
          label="Scan my library periodically"
          help="A safety-net full rescan on a fixed interval, independent of the filesystem watcher. Recommended for libraries on NFS/SMB or in containers, where inotify can miss changes. Takes effect immediately — no restart needed."
          changed={periodicScanChanged}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={periodicScanEnabled} onChange={setPeriodicScanEnabled} />
            <span>Rescan every library on a schedule</span>
          </label>
        </SettingsRow>
        <SettingsRow
          label="Library scan interval"
          help="How often the periodic scan runs. A library is only rescanned if its last scan is older than this, so a recent manual or watcher scan won't be repeated."
          changed={scanIntervalChanged}
        >
          <select
            value={periodicScanFrequency}
            disabled={!periodicScanEnabled}
            onChange={(e) => setPeriodicScanFrequency(e.target.value)}
            className={`${
              scanIntervalChanged ? INPUT_CHANGED_CLASS : INPUT_CLASS
            } disabled:opacity-40`}
          >
            {SCAN_INTERVALS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </SettingsRow>
        <SettingsRow
          label="Empty trash automatically after every scan"
          help="When on, files that vanished from disk are removed from the library immediately after each scan, instead of being kept for 7 days. Leave off if your media lives on a removable or network drive that can go offline — otherwise a temporary disconnect would purge those titles (and their watched state) right away."
          changed={emptyTrashChanged}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={emptyTrashAfterScan} onChange={setEmptyTrashAfterScan} />
            <span>Purge removed files on scan completion</span>
          </label>
        </SettingsRow>
        <SettingsRow
          label="Scanner ffmpeg priority"
          help={
            niceChanged
              ? "Server restart required for changes to take effect."
              : "Wraps background ffmpeg invocations (scanner probes + scheduled tasks) in nice -n N. 0 disables. 10–15 yields politely to live playback. Live transcode sessions always run at normal priority."
          }
          changed={niceChanged}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={19}
              value={scannerNice}
              onChange={(e) => setScannerNice(Number(e.target.value))}
              className={`w-20 tabular-nums ${
                niceChanged ? INPUT_CHANGED_CLASS : INPUT_CLASS
              }`}
            />
            <span className="text-sm text-white/55">nice level</span>
            {niceChanged && <Pill tone="warn">Restart pending</Pill>}
          </div>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Recently Added badge"
        description="How long a freshly-added item shows the red ribbon on its Card."
      >
        <SettingsRow
          label="Window"
          help="0 disables the badge entirely. Default 14 days matches Netflix recency. Takes effect on the next config-poll."
          changed={dirtyFields["Recently-added window"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={365}
              value={recentlyAddedDays}
              onChange={(e) => setRecentlyAddedDays(Number(e.target.value))}
              className={`w-24 tabular-nums ${
                dirtyFields["Recently-added window"]
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }`}
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
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Continue Watching"
        description="Caps on the in-progress rail and the threshold for marking items watched."
      >
        <SettingsRow
          label="Maximum items"
          help="Hard cap on the rail. Plex default 40."
          changed={dirtyFields["CW max items"]}
        >
          <input
            type="number"
            min={1}
            max={200}
            value={cwMaxItems}
            onChange={(e) => setCwMaxItems(Number(e.target.value))}
            className={`w-32 tabular-nums ${
              dirtyFields["CW max items"] ? INPUT_CHANGED_CLASS : INPUT_CLASS
            }`}
            required
          />
        </SettingsRow>
        <SettingsRow
          label="Weeks to consider"
          help="In-progress items last played more than this many weeks ago disappear. 0 disables the time-window filter."
          changed={dirtyFields["CW age weeks"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={520}
              value={cwMaxAgeWeeks}
              onChange={(e) => setCwMaxAgeWeeks(Number(e.target.value))}
              className={`w-24 tabular-nums ${
                dirtyFields["CW age weeks"]
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }`}
              required
            />
            <span className="text-sm text-white/55">weeks</span>
          </div>
        </SettingsRow>
        <SettingsRow
          label="Include season premieres"
          help="When a new season's first episode exists for a show you've watched, surface it on Continue Watching."
          changed={dirtyFields["CW include premieres"]}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={cwIncludePremieres} onChange={setCwIncludePremieres} />
            <span>Include season premieres alongside in-progress items</span>
          </label>
        </SettingsRow>
        <SettingsRow
          label="Video played threshold"
          help="Percentage at which the player auto-scrobbles a session as watched. Same number gates the Continue Watching upper bound so tiles disappear when we mark them played."
          changed={dirtyFields["Watched threshold"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={50}
              max={99}
              value={playedThreshold}
              onChange={(e) => setPlayedThreshold(Number(e.target.value))}
              className={`w-24 tabular-nums ${
                dirtyFields["Watched threshold"]
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }`}
              required
            />
            <span className="text-sm text-white/55">%</span>
          </div>
        </SettingsRow>
        <SettingsRow
          label="Video play completion"
          help="What counts as 'watched'. first_credits_marker requires that the detect_markers task has found one; falls back to the percentage threshold otherwise. earliest_of_both scrobbles at whichever lands first."
          changed={dirtyFields["Completion behaviour"]}
        >
          <select
            value={completionBehaviour}
            onChange={(e) =>
              setCompletionBehaviour(
                e.target.value as typeof completionBehaviour,
              )
            }
            className={`w-full ${
              dirtyFields["Completion behaviour"]
                ? INPUT_CHANGED_CLASS
                : INPUT_CLASS
            }`}
          >
            <option value="threshold_pct">Percentage threshold only</option>
            <option value="first_credits_marker">
              First credits marker (fall back to threshold)
            </option>
            <option value="earliest_of_both">
              Earliest of either (recommended once markers are populated)
            </option>
          </select>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Metadata"
        description="Preferred language for descriptions, taglines, and titles. Honored by every metadata agent that supports it (TMDB, TheTVDB, AniList)."
      >
        <SettingsRow
          label="Metadata language"
          help={
            metadataLanguageChanged
              ? "Server restart required for changes to take effect. Existing items keep their current text — run Refresh metadata on a library (or per-item) to re-pull in the new language."
              : "BCP-47 tag (e.g. en-US, ja-JP). TMDB honors it directly; TheTVDB picks the closest ISO 639-3 endpoint; AniList prefers English on en-* locales and Japanese on ja-* locales. OMDb is always English."
          }
          changed={metadataLanguageChanged}
        >
          <div className="flex flex-wrap items-center gap-2">
            <select
              value={
                METADATA_LANGUAGES.some((l) => l.tag === metadataLanguage)
                  ? metadataLanguage
                  : "__custom"
              }
              onChange={(e) => {
                if (e.target.value !== "__custom") {
                  setMetadataLanguage(e.target.value);
                }
              }}
              className={
                metadataLanguageChanged ? INPUT_CHANGED_CLASS : INPUT_CLASS
              }
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
              className={`w-28 font-mono ${
                metadataLanguageChanged
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }`}
            />
            {metadataLanguageChanged && (
              <Pill tone="warn">Restart pending</Pill>
            )}
          </div>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Audio"
        description="EBU R 128 loudness normalization."
      >
        <SettingsRow
          label="Normalize loudness by default"
          help="When on, ffmpeg's loudnorm filter is applied to every session — uses stored per-file measurements when the analyze_loudness task has run, otherwise generic targets. Users can still toggle off per-session via the player audio menu."
          changed={dirtyFields["Audio normalize"]}
        >
          <label className="inline-flex items-center gap-2 text-sm">
            <Switch checked={audioNormalize} onChange={setAudioNormalize} />
            <span>Normalize loudness on every transcode session</span>
          </label>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Subtitles"
        description="Server-wide adjustments that apply to every session."
      >
        <SettingsRow
          label="Default sync offset"
          help="Added to every session's subtitle offset before the WebVTT cue shift. Use this when your library has a consistent cross-title drift (common with anime fansub re-encodes whose ASS tracks were authored against a slightly differently-cut master). Positive = subs render later; negative = earlier. The player's per-file stepper still works as a relative tweak on top of this baseline. 0 disables (the previous behaviour)."
          changed={dirtyFields["Subtitle default offset"]}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={-30000}
              max={30000}
              step={100}
              value={subtitleDefaultOffsetMs}
              onChange={(e) =>
                setSubtitleDefaultOffsetMs(Number(e.target.value))
              }
              className={`w-28 tabular-nums ${
                dirtyFields["Subtitle default offset"]
                  ? INPUT_CHANGED_CLASS
                  : INPUT_CLASS
              }`}
              required
            />
            <span className="text-sm text-white/55">
              ms ({(subtitleDefaultOffsetMs / 1000).toFixed(2)} s)
            </span>
          </div>
        </SettingsRow>
      </SettingsCard>

      <SettingsCard
        title="Database"
        description="SQLite tuning. Increase the page cache for libraries with tens of thousands of items."
      >
        <SettingsRow
          label="Page cache size"
          help={
            dbCacheChanged
              ? "Server restart required for changes to take effect."
              : "0 = SQLite default (~2 MiB). 64 MiB is a sensible baseline for a busy library."
          }
          changed={dbCacheChanged}
        >
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              max={4096}
              value={dbCacheMb}
              onChange={(e) => setDbCacheMb(Number(e.target.value))}
              className={`w-24 tabular-nums ${
                dbCacheChanged ? INPUT_CHANGED_CLASS : INPUT_CLASS
              }`}
              required
            />
            <span className="text-sm text-white/55">MiB</span>
            {dbCacheChanged && <Pill tone="warn">Restart pending</Pill>}
          </div>
        </SettingsRow>
      </SettingsCard>

      <SaveBar
        dirtyCount={dirtyCount}
        summary={dirtyLabels.slice(0, 3).join(", ") +
          (dirtyLabels.length > 3 ? `, +${dirtyLabels.length - 3} more` : "")}
        onSave={save}
        onDiscard={discard}
      />
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
