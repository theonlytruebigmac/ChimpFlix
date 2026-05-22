//! Filename and path parsing for the scanner.
//!
//! Pure, regex-based, no I/O. Tested by unit tests at the bottom of the
//! file — they document every pattern the scanner is expected to handle.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::models::{LibraryKind, make_sort_title};

#[derive(Debug, Clone)]
pub enum Classification {
    Movie {
        title: String,
        sort_title: String,
        year: Option<i32>,
    },
    Episode {
        show_title: String,
        show_sort_title: String,
        show_year: Option<i32>,
        season: i32,
        episode: i32,
        title: Option<String>,
        /// When the file was absolute-numbered (anime style with no
        /// S/E tag), the raw on-disk number. The dispatcher may later
        /// remap (season, episode) to season-relative form, but we
        /// preserve the absolute number so absolute-aware metadata
        /// agents can look up by their native numbering.
        absolute_number: Option<i32>,
    },
}

/// Classifier result. `auto_matched = true` when the parser pulled
/// real metadata out of the path; `false` when we fell back to a
/// best-effort filename cleanup so the scanner can still link the
/// file. The scanner persists this flag onto `items.auto_matched`
/// so an "Unmatched files" admin view can surface stubs for manual
/// fix-matching without having to rename files on disk.
#[derive(Debug, Clone)]
pub struct ClassifyResult {
    pub class: Classification,
    pub auto_matched: bool,
}

/// Classify a media file relative to a library root path.
///
/// Always succeeds: when the kind-specific regex pipeline returns
/// `None`, this falls back to a "best effort" Classification built
/// from the filename + parent directory, with `auto_matched = false`
/// so the UI can flag it. Pre-this-change the scanner silently
/// dropped any file the parser couldn't fingerprint, which on a
/// 1584-file anime library with non-standard release names meant
/// ~89% of media never appeared in the catalog.
pub fn classify(file_path: &Path, root: &Path, kind: LibraryKind) -> ClassifyResult {
    let confident = match kind {
        LibraryKind::Movies => classify_movie(file_path),
        LibraryKind::Shows => classify_episode(file_path, root),
        LibraryKind::Anime => classify_anime(file_path, root),
    };
    if let Some(class) = confident {
        return ClassifyResult {
            class,
            auto_matched: true,
        };
    }
    ClassifyResult {
        class: fallback_classification(file_path, root, kind),
        auto_matched: false,
    }
}

/// Build a placeholder Classification from `file_path` when the
/// regex-driven path failed. Movie libraries get a Movie row keyed
/// on the cleaned filename; show/anime libraries get an Episode
/// row under a "show" derived from the parent dir + a synthetic
/// season=1/episode=<sequential> so the scanner's `upsert_episode`
/// conflict key (season_id, episode_number) doesn't collide across
/// many unmatched files in the same show.
///
/// The episode number is derived from a 16-bit hash of the file's
/// path relative to the library root. Stable across rescans (same
/// path → same number) and distributes uniformly enough that
/// collisions among the few hundred unmatched files per show are
/// vanishingly rare. Real metadata will overwrite this once the
/// operator fix-matches.
fn fallback_classification(file_path: &Path, root: &Path, kind: LibraryKind) -> Classification {
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    let cleaned_title = clean_filename_for_title(stem);
    let title = if cleaned_title.is_empty() {
        "Untitled".to_string()
    } else {
        cleaned_title
    };

    match kind {
        LibraryKind::Movies => {
            // For a movies library, every file is its own item.
            Classification::Movie {
                sort_title: make_sort_title(&title),
                title,
                year: None,
            }
        }
        LibraryKind::Shows | LibraryKind::Anime => {
            // Use the first path component below the root as the
            // show. With Sonarr's `<root>/<show>/<season>/<file>`
            // layout that gives the show name; with a flat
            // `<root>/<file>` layout (no parent dir at all) it
            // falls back to the cleaned filename so we still
            // produce a non-empty title.
            let rel = file_path.strip_prefix(root).ok();
            let show_dir = rel
                .and_then(|r| r.components().next().map(|c| c.as_os_str().to_owned()))
                .and_then(|n| n.into_string().ok());
            let (show_title, show_year) = match show_dir.as_deref() {
                Some(d) if !d.is_empty() && d != stem => {
                    parse_title_with_year(d).unwrap_or_else(|| (d.to_string(), None))
                }
                _ => (title.clone(), None),
            };

            // Deterministic episode number from a stable hash of
            // the relative path. Keeps the upsert key unique per
            // file without colliding with real episodes (which
            // tend to be small numbers) by biasing into the
            // 32768..=65535 range.
            let path_for_hash = file_path
                .strip_prefix(root)
                .unwrap_or(file_path)
                .to_string_lossy();
            let h: u16 = path_for_hash
                .bytes()
                .fold(0u16, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u16));
            let episode = 0x8000i32 | (h as i32 & 0x7FFF);

            Classification::Episode {
                show_sort_title: make_sort_title(&show_title),
                show_title,
                show_year,
                season: 1,
                episode,
                title: Some(title),
                // Fallback rows use a hash-based pseudo-episode-number; not
                // a real absolute number, so leave None.
                absolute_number: None,
            }
        }
    }
}

/// Turn a filename stem into a presentable title: drop bracketed
/// release tags, replace `.` / `_` with spaces, collapse whitespace.
/// Best-effort; the operator can rename via fix-match.
fn clean_filename_for_title(stem: &str) -> String {
    let unbracketed = strip_brackets(stem);
    let spaced: String = unbracketed
        .chars()
        .map(|c| match c {
            '.' | '_' => ' ',
            _ => c,
        })
        .collect();
    // Collapse runs of whitespace and trim.
    let mut out = String::with_capacity(spaced.len());
    let mut prev_ws = true; // strips leading whitespace
    for c in spaced.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(c);
            prev_ws = false;
        }
    }
    out.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Movies
// ---------------------------------------------------------------------------

fn classify_movie(file_path: &Path) -> Option<Classification> {
    let stem = file_path.file_stem()?.to_str()?;
    let parent_name = file_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Prefer the parent dir name when it has a year — handles the
    // "Movie (Year)/Movie.mkv" layout where the year is on the folder.
    let parent_parsed = parse_title_with_year(parent_name);
    let stem_parsed = parse_title_with_year(stem);
    let (title, year) = match (&parent_parsed, &stem_parsed) {
        (Some((t, Some(y))), _) => (t.clone(), Some(*y)),
        (_, Some((t, year))) => (t.clone(), *year),
        (Some((t, None)), None) => (t.clone(), None),
        (None, None) => return None,
    };

    if title.is_empty() {
        return None;
    }

    Some(Classification::Movie {
        sort_title: make_sort_title(&title),
        title,
        year,
    })
}

// ---------------------------------------------------------------------------
// Episodes
// ---------------------------------------------------------------------------

static EP_TAG: LazyLock<Regex> = LazyLock::new(|| {
    // Matches S01E02 / s1e3 / S01E001 etc.
    Regex::new(r"(?i)\bS(?P<s>\d{1,2})E(?P<e>\d{1,3})\b").unwrap()
});

static EP_ALT_TAG: LazyLock<Regex> = LazyLock::new(|| {
    // Matches "1x02" style.
    Regex::new(r"(?i)\b(?P<s>\d{1,2})x(?P<e>\d{1,3})\b").unwrap()
});

fn classify_episode(file_path: &Path, root: &Path) -> Option<Classification> {
    // First path component below the library root is the show directory.
    let rel = file_path.strip_prefix(root).ok()?;
    let mut comps = rel.components();
    let show_dir = comps.next()?.as_os_str().to_str()?;

    let (show_title_raw, show_year) =
        parse_title_with_year(show_dir).unwrap_or_else(|| (show_dir.to_string(), None));

    let stem = file_path.file_stem()?.to_str()?;
    let (season, episode, remainder) = parse_season_episode(stem)?;

    let episode_title = remainder
        .as_ref()
        .map(|r| sanitize_title(r.trim_start_matches(|c: char| c == '-' || c.is_whitespace())))
        .filter(|s| !s.is_empty());

    Some(Classification::Episode {
        show_sort_title: make_sort_title(&show_title_raw),
        show_title: show_title_raw,
        show_year,
        season,
        episode,
        title: episode_title,
        // S/E-tagged paths are season-relative by construction; no
        // absolute number to preserve.
        absolute_number: None,
    })
}

/// From an episode filename stem, extract (season, episode, remainder).
/// `remainder` is the text after the season/episode tag, useful as a
/// fallback episode title when TMDB lookup fails.
fn parse_season_episode(stem: &str) -> Option<(i32, i32, Option<String>)> {
    if let Some(caps) = EP_TAG.captures(stem) {
        let s: i32 = caps["s"].parse().ok()?;
        let e: i32 = caps["e"].parse().ok()?;
        let after = &stem[caps.get(0)?.end()..];
        return Some((s, e, Some(after.to_string()).filter(|s| !s.is_empty())));
    }
    if let Some(caps) = EP_ALT_TAG.captures(stem) {
        let s: i32 = caps["s"].parse().ok()?;
        let e: i32 = caps["e"].parse().ok()?;
        let after = &stem[caps.get(0)?.end()..];
        return Some((s, e, Some(after.to_string()).filter(|s| !s.is_empty())));
    }
    None
}

// ---------------------------------------------------------------------------
// Anime episodes
// ---------------------------------------------------------------------------
//
// Anime libraries differ from generic shows in two ways:
//
//   1. Filenames usually carry an *absolute* episode number rather than
//      season+episode. We map these onto season 1 so the existing schema
//      (which requires a season_id) just works; the AniList agent will
//      eventually split long-runners back into proper seasons during
//      enrichment.
//
//   2. Fansub release tags live in the filename: `[Group] Title - 12.mkv`,
//      `[SubsPlease] Title - 234 (1080p).mkv`, and so on. We use the
//      parent directory as the canonical show title and only look to the
//      filename for the episode number.

static ANIME_EPISODE_NUM: LazyLock<Regex> = LazyLock::new(|| {
    // Episode number that's either:
    //   - "- 12" / " - 12" / "-12" / ".12" — common after fansub tags
    //     or Sonarr-style dot/dash separators. We accept dot, dash,
    //     and underscore as the leading separator since releases like
    //     `Show.Name.01.1080p.mkv` are common enough to warrant it.
    //   - "EP12", "Ep 12", "E12" — no season marker, just episode.
    //   - bare "12" at the end of the stem.
    // Optional version suffix (v2, v3) is allowed everywhere. Order
    // matters — separator-anchored forms are more specific and must
    // match first or the bare-trailing pattern would over-grab on
    // releases with multiple digit clusters.
    Regex::new(
        r"(?ix)
        (?:
            [\s._-]+ -? \s* (?P<ep1>\d{1,4})(?:v\d+)?\b
          | \bEP?\s*(?P<ep2>\d{1,4})(?:v\d+)?\b
          | \b(?P<ep3>\d{1,4})(?:v\d+)?\s*$
        )",
    )
    .unwrap()
});

fn classify_anime(file_path: &Path, root: &Path) -> Option<Classification> {
    // Prefer the standard S01E02 path when it's present — many users
    // organize anime that way, and we want to honor it.
    if let Some(c) = classify_episode(file_path, root) {
        return Some(c);
    }

    // Otherwise: the first path component below the root is the show
    // directory; the filename carries an absolute episode number we map
    // to season 1.
    let rel = file_path.strip_prefix(root).ok()?;
    let mut comps = rel.components();
    let show_dir = comps.next()?.as_os_str().to_str()?;
    let (show_title_raw, show_year) =
        parse_title_with_year(show_dir).unwrap_or_else(|| (show_dir.to_string(), None));

    let stem = file_path.file_stem()?.to_str()?;
    // Strip fansub-tag brackets before regex matching so the trailing
    // "[1080p]" or "[CRC32]" can't be misread as the episode number.
    let cleaned = strip_brackets(stem);
    let cleaned = cleaned.trim();
    let caps = ANIME_EPISODE_NUM.captures(cleaned)?;
    let episode: i32 = caps
        .name("ep1")
        .or_else(|| caps.name("ep2"))
        .or_else(|| caps.name("ep3"))?
        .as_str()
        .parse()
        .ok()?;

    Some(Classification::Episode {
        show_sort_title: make_sort_title(&show_title_raw),
        show_title: show_title_raw,
        show_year,
        season: 1,
        episode,
        title: None,
        // Anime bare-number path — the on-disk number IS the absolute
        // number. Preserve it so a later resolver can remap into
        // (season, episode) once season episode-counts are known.
        absolute_number: Some(episode),
    })
}

// ---------------------------------------------------------------------------
// Shared title parsing
// ---------------------------------------------------------------------------

static TITLE_YEAR: LazyLock<Regex> = LazyLock::new(|| {
    // Title + 4-digit year. Year can be in parens, brackets, surrounded by
    // dots, or whitespace-delimited.
    Regex::new(
        r"(?x)
        ^
        (?P<title>.+?)
        [\s.\-_(\[]+
        (?P<year>(?:19|20)\d{2})
        [\s.\-_)\]]*
        ",
    )
    .unwrap()
});

pub fn parse_title_with_year(s: &str) -> Option<(String, Option<i32>)> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(caps) = TITLE_YEAR.captures(trimmed) {
        let title = sanitize_title(&caps["title"]);
        let year: Option<i32> = caps["year"].parse().ok();
        if !title.is_empty() {
            return Some((title, year));
        }
    }
    Some((sanitize_title(trimmed), None))
}

/// Public wrapper around [`sanitize_title`] for one-shot title heals
/// (e.g. `queries::heal_filename_derived_episode_titles`). The parser
/// itself only consumes the function internally; exposing it as
/// `pub` lets the library crate run the same sanitization pass on
/// rows that were upserted before the sanitize logic shipped.
pub fn sanitize_title_pub(s: &str) -> String {
    sanitize_title(s)
}

fn sanitize_title(s: &str) -> String {
    let replaced = s.replace(['.', '_'], " ");
    let cleaned = strip_brackets(&replaced);
    let collapsed: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    // Pass order matters:
    //  1. Quality tokens FIRST so "WEB-DL" and "x265" are consumed as
    //     whole tokens before the trailing-release-group pass sees the
    //     internal dash in "WEB-DL" and mistakes "-DL" for a tag.
    //  2. Trailing release-group ("Day -Kitsune") second.
    //  3. Leading absolute-ep prefix ("013 - ") last.
    let collapsed = strip_quality_tokens(&collapsed);
    let collapsed = strip_trailing_release_group(&collapsed);
    let collapsed = strip_leading_absolute_ep(&collapsed);
    collapsed.trim().to_string()
}

/// Strip a trailing release-group tag like " -Kitsune".
///
/// Conservative — only fires when:
///   - The last dash is preceded by whitespace (or starts the string).
///     This preserves kebab-case titles like "Self-Titled" and
///     "X-Ray Vision".
///   - The dash is NOT followed by whitespace (kills "Mockingjay - Part 1"
///     as a false positive).
///   - The trailing token starts with an uppercase letter or digit
///     (so all-lowercase trailing fragments are preserved).
fn strip_trailing_release_group(s: &str) -> String {
    let trimmed = s.trim_end();
    if let Some(idx) = trimmed.rfind('-') {
        let after = &trimmed[idx + 1..];
        if after.is_empty() || after.contains(char::is_whitespace) {
            return trimmed.to_string();
        }
        let first = after.chars().next().unwrap();
        if !(first.is_ascii_uppercase() || first.is_ascii_digit()) {
            return trimmed.to_string();
        }
        // Require whitespace (or start-of-string) immediately before the
        // dash so kebab-case titles like "Self-Titled" are preserved.
        let before_char = trimmed[..idx].chars().last();
        if before_char.is_some_and(|c| !c.is_whitespace()) {
            return trimmed.to_string();
        }
        return trimmed[..idx].trim_end().to_string();
    }
    trimmed.to_string()
}

/// Strip a leading absolute-episode-number prefix like "013 - ".
fn strip_leading_absolute_ep(s: &str) -> String {
    let trimmed = s.trim_start();
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if !(2..=4).contains(&i) {
        return trimmed.to_string();
    }
    let rest = &trimmed[i..];
    // Accept "13 - Title" / "013 Title" / "13- Title" / "013-Title".
    let after_dash = rest
        .trim_start()
        .strip_prefix('-')
        .or_else(|| Some(rest.trim_start()))
        .map(str::trim_start);
    match after_dash {
        Some(remaining) if !remaining.is_empty() => remaining.to_string(),
        _ => trimmed.to_string(),
    }
}

const QUALITY_TOKENS: &[&str] = &[
    "1080p", "720p", "2160p", "480p", "4k", "uhd", "hdr", "hdr10", "dv",
    "web-dl", "webdl", "webrip", "bluray", "blu-ray", "bdrip", "dvdrip",
    "hevc", "x265", "x264", "h.264", "h264", "h.265", "h265", "av1",
    "10bit", "8bit", "aac", "ac3", "eac3", "flac", "dts", "ddp5.1",
    "ddp", "remux", "rerip", "proper", "repack",
];

/// Drop quality / codec tokens that occasionally appear unbracketed in
/// release filenames. Case-insensitive token match; preserves any token
/// that isn't recognized so real titles like "1080" (rare but possible)
/// are unaffected unless they're a recognized quality marker.
fn strip_quality_tokens(s: &str) -> String {
    let kept: Vec<&str> = s
        .split_whitespace()
        .filter(|tok| {
            let lower = tok.to_ascii_lowercase();
            !QUALITY_TOKENS.contains(&lower.as_str())
        })
        .collect();
    kept.join(" ")
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth_paren = 0u32;
    let mut depth_bracket = 0u32;
    for ch in s.chars() {
        match ch {
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            '[' => depth_bracket += 1,
            ']' => depth_bracket = depth_bracket.saturating_sub(1),
            _ if depth_paren == 0 && depth_bracket == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Video extensions
// ---------------------------------------------------------------------------

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "webm", "mpg", "mpeg", "m2ts", "ts", "wmv", "flv", "ogv",
];

pub fn is_video_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    VIDEO_EXTENSIONS.iter().any(|v| *v == ext)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn movie_with_paren_year() {
        let p = PathBuf::from("/m/Arrival (2016).mkv");
        match classify(&p, Path::new("/m"), LibraryKind::Movies).class {
            Classification::Movie { title, year, .. } => {
                assert_eq!(title, "Arrival");
                assert_eq!(year, Some(2016));
            }
            _ => panic!("expected movie"),
        }
    }

    #[test]
    fn movie_dot_separated() {
        let p = PathBuf::from("/m/The.Matrix.1999.1080p.WEBRip.x264.mkv");
        match classify(&p, Path::new("/m"), LibraryKind::Movies).class {
            Classification::Movie {
                title,
                year,
                sort_title,
                ..
            } => {
                assert_eq!(title, "The Matrix");
                assert_eq!(sort_title, "Matrix");
                assert_eq!(year, Some(1999));
            }
            _ => panic!("expected movie"),
        }
    }

    #[test]
    fn movie_inside_year_folder() {
        let p = PathBuf::from("/m/Arrival (2016)/Arrival.mkv");
        match classify(&p, Path::new("/m"), LibraryKind::Movies).class {
            Classification::Movie { title, year, .. } => {
                assert_eq!(title, "Arrival");
                assert_eq!(year, Some(2016));
            }
            _ => panic!("expected movie"),
        }
    }

    #[test]
    fn movie_no_year() {
        let p = PathBuf::from("/m/Untitled.mkv");
        match classify(&p, Path::new("/m"), LibraryKind::Movies).class {
            Classification::Movie { title, year, .. } => {
                assert_eq!(title, "Untitled");
                assert_eq!(year, None);
            }
            _ => panic!("expected movie"),
        }
    }

    #[test]
    fn episode_seasons_dir() {
        let p =
            PathBuf::from("/s/Severance/Season 01/Severance - S01E01 - Good News About Hell.mkv");
        match classify(&p, Path::new("/s"), LibraryKind::Shows).class {
            Classification::Episode {
                show_title,
                season,
                episode,
                title,
                ..
            } => {
                assert_eq!(show_title, "Severance");
                assert_eq!(season, 1);
                assert_eq!(episode, 1);
                assert_eq!(title.as_deref(), Some("Good News About Hell"));
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn episode_dot_form_alt_tag() {
        let p = PathBuf::from("/s/Battlestar Galactica (2004)/Battlestar.Galactica.1x05.mkv");
        match classify(&p, Path::new("/s"), LibraryKind::Shows).class {
            Classification::Episode {
                show_title,
                show_year,
                season,
                episode,
                ..
            } => {
                assert_eq!(show_title, "Battlestar Galactica");
                assert_eq!(show_year, Some(2004));
                assert_eq!(season, 1);
                assert_eq!(episode, 5);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn no_episode_tag_now_falls_back_unmatched() {
        // Pre-phase-68 the scanner silently dropped files like this.
        // New contract: the parser ALWAYS returns a Classification;
        // names it couldn't fingerprint come back with
        // `auto_matched = false` so the scanner still links them and
        // the operator can fix-match later.
        let p = PathBuf::from("/s/Severance/randomfile.mkv");
        let res = classify(&p, Path::new("/s"), LibraryKind::Shows);
        assert!(!res.auto_matched);
        match res.class {
            Classification::Episode { show_title, .. } => {
                assert_eq!(show_title, "Severance");
            }
            _ => panic!("expected an episode stub"),
        }
    }

    #[test]
    fn anime_fansub_with_dash() {
        let p = PathBuf::from(
            "/a/Frieren Beyond Journey's End/[SubsPlease] Frieren - 28 (1080p) [ABCD1234].mkv",
        );
        match classify(&p, Path::new("/a"), LibraryKind::Anime).class {
            Classification::Episode {
                show_title,
                season,
                episode,
                ..
            } => {
                assert_eq!(show_title, "Frieren Beyond Journey's End");
                assert_eq!(season, 1);
                assert_eq!(episode, 28);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_version_suffix_stripped() {
        let p = PathBuf::from("/a/Bocchi the Rock/Bocchi - 12v2.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).class {
            Classification::Episode { episode, .. } => assert_eq!(episode, 12),
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_absolute_long_runner() {
        // One Piece-style absolute numbering past 1000.
        let p = PathBuf::from("/a/One Piece/One Piece - 1100.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).class {
            Classification::Episode {
                season, episode, ..
            } => {
                assert_eq!(season, 1);
                assert_eq!(episode, 1100);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_absolute_captures_absolute_number() {
        // Bare-number anime files preserve the absolute number for
        // later season-relative remapping.
        let p = PathBuf::from("/a/Frieren/[SubsPlease] Frieren - 29.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).class {
            Classification::Episode {
                absolute_number,
                episode,
                season,
                ..
            } => {
                assert_eq!(absolute_number, Some(29));
                assert_eq!(episode, 29);
                assert_eq!(season, 1);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn s_e_tagged_episode_has_no_absolute_number() {
        // When the file carries a real S/E tag, absolute_number stays
        // None — the file already says what season + episode it is.
        let p = PathBuf::from("/a/Attack on Titan/Season 4/Attack.on.Titan.S04E28.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).class {
            Classification::Episode {
                absolute_number, ..
            } => {
                assert_eq!(absolute_number, None);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_with_season_episode_tag_uses_show_parser() {
        // When the file does carry S01E05, prefer that over the absolute
        // path so users who organize anime by season aren't surprised.
        let p = PathBuf::from("/a/Attack on Titan/Season 4/Attack.on.Titan.S04E28.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).class {
            Classification::Episode {
                season, episode, ..
            } => {
                assert_eq!(season, 4);
                assert_eq!(episode, 28);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_no_episode_number_falls_back_unmatched() {
        // Same contract as the Shows fallback test above — the
        // anime classifier never returns None either; failed-regex
        // files get `auto_matched = false`.
        let p = PathBuf::from("/a/Bleach/notes.mkv");
        let res = classify(&p, Path::new("/a"), LibraryKind::Anime);
        assert!(!res.auto_matched);
        match res.class {
            Classification::Episode { show_title, .. } => {
                assert_eq!(show_title, "Bleach");
            }
            _ => panic!("expected an episode stub"),
        }
    }

    #[test]
    fn extension_check() {
        assert!(is_video_file(Path::new("a.MKV")));
        assert!(is_video_file(Path::new("a.mp4")));
        assert!(!is_video_file(Path::new("a.txt")));
        assert!(!is_video_file(Path::new("a")));
    }

    #[test]
    fn sort_title_strips_articles() {
        assert_eq!(make_sort_title("The Matrix"), "Matrix");
        assert_eq!(make_sort_title("A Quiet Place"), "Quiet Place");
        assert_eq!(make_sort_title("An Education"), "Education");
        assert_eq!(make_sort_title("Inception"), "Inception");
    }

    #[test]
    fn sanitize_strips_trailing_release_group() {
        assert_eq!(sanitize_title("Barrier Day -Kitsune"), "Barrier Day");
        assert_eq!(
            sanitize_title("The Party from Hell Begins -Kitsune"),
            "The Party from Hell Begins"
        );
        assert_eq!(sanitize_title("The First Bloom -ToonsHub"), "The First Bloom");
        assert_eq!(
            sanitize_title("The Day of Departure -AnoZu"),
            "The Day of Departure"
        );
    }

    #[test]
    fn sanitize_strips_leading_absolute_ep_prefix() {
        assert_eq!(sanitize_title("013 - Barrier Day"), "Barrier Day");
        assert_eq!(sanitize_title("01 - Pilot"), "Pilot");
        assert_eq!(sanitize_title("014 The Party"), "The Party");
        assert_eq!(sanitize_title("13-Pilot"), "Pilot");
    }

    #[test]
    fn sanitize_strips_quality_tokens() {
        assert_eq!(sanitize_title("Pilot 1080p WEB-DL"), "Pilot");
        assert_eq!(sanitize_title("Pilot 2160p HEVC x265"), "Pilot");
        assert_eq!(sanitize_title("Pilot BluRay 10bit"), "Pilot");
    }

    #[test]
    fn sanitize_preserves_real_dash_titles() {
        // "Mockingjay - Part 1" has whitespace after the dash → not a tag.
        assert_eq!(sanitize_title("Mockingjay - Part 1"), "Mockingjay - Part 1");
        assert_eq!(sanitize_title("Self-Titled"), "Self-Titled");
    }

    #[test]
    fn sanitize_combines_all_fixes_on_real_anime_filename() {
        assert_eq!(
            sanitize_title("013 - Barrier Day -Kitsune 1080p WEB-DL"),
            "Barrier Day"
        );
    }
}
