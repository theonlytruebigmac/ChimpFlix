# Intro & credits detection

ChimpFlix automatically finds the intro and credits in every video so the
player can show a **Skip Intro / Skip Credits** button. Detection runs in
the background via the `detect_markers` scheduled task and is also reachable
from the **Detect markers** action on each item, library, and the
**Library → Bulk operations** screen.

## How detection works

Three cooperating layers, in increasing order of accuracy:

1. **`blackdetect`** — ffmpeg scans for sustained black frames and labels
   the first long black run before 600 s as an intro (post-cold-open
   fade-to-black) and any black run in the last ~8% of the file as the
   credits. Works on any source. Heuristic; can mislabel mid-film fades
   on movies.

2. **Chapter metadata** — ffprobe lists embedded chapters. Titles
   matching `intro / opening / opening credits / opening theme` (within
   the first 600 s) become intro markers; `end credits / closing credits /
   outro / credits` (in the last 30%) become credits markers. Authoritative
   when present — Bluray rips and well-mastered MKVs usually have these.
   Chapter-derived markers always win over `blackdetect` for the same
   kind.

3. **Audio fingerprint match** — chromaprint, computed via
   [`rusty-chromaprint`](https://crates.io/crates/rusty-chromaprint). The
   server stores a canonical intro fingerprint per show; on every other
   episode of that show the detector extracts the first ~10 min of audio
   and searches for the fingerprint. A confident match anchors the intro
   start/end far more accurately than either heuristic above.

## Where fingerprints come from

Two paths, no operator action required for either:

- **Auto-capture from chapters.** When detection finds an intro from a
  chapter title (layer 2 above), the server captures a fingerprint from
  that chapter's audio range and stores it under the show as
  `captured_by = 'auto'`. The next episode in the same show uses it.
  Bluray-rip libraries seed themselves with zero operator input.

- **Manual capture.** When an operator saves a manual `intro` marker via
  the per-episode **Edit markers…** dialog (visible on every episode row
  to owners), the server captures from that audio range and stores
  `captured_by = 'manual'`. Manual fingerprints always win over auto —
  a manual save replaces any existing capture for the show.

Either path makes E02+ "skip intro" land accurately without further
input.

## Operator workflow

The flow most operators want:

1. Browse to a show. Open the show modal.
2. Run **Detect markers** from the admin action menu (or wait for the
   scheduled task on its own cadence).
3. If the show has chapter metadata, the intro is auto-anchored and a
   fingerprint is captured silently. You're done.
4. If the show has no chapter metadata, the first episode lands with
   `blackdetect`'s guess. Click **Edit markers** on episode 1, adjust the
   intro start/end to the actual theme song, and save. The fingerprint
   is captured in the background.
5. Run **Detect markers** again on the rest of the show (item-level
   detection re-runs on every episode). Each episode's intro is now
   audio-matched against the canonical fingerprint.

## Reviewing captures

The **Maintenance → Intro fingerprints** admin page lists every captured
fingerprint with source (manual vs auto), capture timestamp, and a
**Clear** button. Clearing forces the next detect-markers run to
re-derive the intro from scratch (or auto-capture from chapter metadata
if available). Useful when the original capture was from a bad intro
range — clear, re-mark a better range, save.

The per-episode **Edit markers** dialog also shows a fingerprint badge
inline with capture date and a clear button scoped to that show, so the
operator can reset without leaving the editor.

## Threshold tuning

The match threshold is `DEFAULT_MATCH_THRESHOLD = 14.4` average bits
flipped per 32-bit chromaprint frame — the chromaprint paper's
"confident match" boundary. This is the default for every library and
should work well for most content. If your library has noisy audio
(VHS rips, dub-over languages), expect some false-negative misses
where the match falls short of the threshold and the detector falls
back to `blackdetect`. The constant is in
`crates/transcoder/src/fingerprint.rs` — bump it to 16-18 to accept
noisier matches, drop to 10-12 if you're seeing false positives.

## Limitations

- **One fingerprint per show.** Per-season storage is plumbed but
  unused today — shows whose theme music changes across seasons will
  only match the season that seeded the capture. Workaround: clear the
  fingerprint, switch seasons, re-mark.
- **No fingerprint for credits.** The credits range is much more
  variable per episode (post-credits scenes, blooper reels, etc.) and
  fingerprinting it tends to false-positive into the body of an
  episode. Credits detection stays heuristic.
- **Multi-version movies.** The marker editor only opens the first
  media file for an item. Movies with both 4K and 1080p versions can
  only have markers on the primary file.
- **No keyboard shortcut to skip.** The on-screen pill is the only
  affordance today.

## Where to look in the code

- `crates/transcoder/src/markers.rs` — blackdetect + chapter detection,
  `DetectedMarker { source, signature_range }`.
- `crates/transcoder/src/fingerprint.rs` — chromaprint extract + match,
  with synthetic-buffer unit tests.
- `crates/library/migrations/20260519010000_phase56_show_intro_fingerprints.sql`
  — schema.
- `crates/library/src/queries.rs` — `upsert_show_intro_fingerprint`,
  `get_show_intro_fingerprint`, `list_all_show_intro_fingerprints`.
- `crates/server/src/api/markers.rs` — capture + override helpers and
  the per-media-file marker CRUD endpoints.
- `crates/server/src/api/admin/fingerprints.rs` — admin listing + clear.
- `web/src/components/MarkerEditor.tsx` — operator editor + fingerprint
  badge.
- `web/src/components/admin/AdminIntroFingerprintsClient.tsx` — admin
  listing UI.
