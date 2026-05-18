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
    },
}

/// Classify a media file relative to a library root path.
pub fn classify(file_path: &Path, root: &Path, kind: LibraryKind) -> Option<Classification> {
    match kind {
        LibraryKind::Movies => classify_movie(file_path),
        LibraryKind::Shows => classify_episode(file_path, root),
        LibraryKind::Anime => classify_anime(file_path, root),
    }
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
    //   - "- 12", "- 12v2" (the most common form after fansub tags)
    //   - "EP12", "Ep 12", "E12" (no season marker)
    //   - bare "12" at the end of the stem
    // We require at least one digit and allow an optional version suffix
    // (e.g. v2). Order matters — the dash-anchored form is more specific
    // and must match first.
    Regex::new(
        r"(?ix)
        (?:
            \s-\s*(?P<ep1>\d{1,4})(?:v\d+)?\b
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

fn sanitize_title(s: &str) -> String {
    let replaced = s.replace(['.', '_'], " ");
    // Strip noisy bracket/paren groups (release tags, codec info, etc.)
    let cleaned = strip_brackets(&replaced);
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
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
        match classify(&p, Path::new("/m"), LibraryKind::Movies).unwrap() {
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
        match classify(&p, Path::new("/m"), LibraryKind::Movies).unwrap() {
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
        match classify(&p, Path::new("/m"), LibraryKind::Movies).unwrap() {
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
        match classify(&p, Path::new("/m"), LibraryKind::Movies).unwrap() {
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
        match classify(&p, Path::new("/s"), LibraryKind::Shows).unwrap() {
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
        match classify(&p, Path::new("/s"), LibraryKind::Shows).unwrap() {
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
    fn rejects_no_episode_tag() {
        let p = PathBuf::from("/s/Severance/randomfile.mkv");
        assert!(classify(&p, Path::new("/s"), LibraryKind::Shows).is_none());
    }

    #[test]
    fn anime_fansub_with_dash() {
        let p =
            PathBuf::from("/a/Frieren Beyond Journey's End/[SubsPlease] Frieren - 28 (1080p) [ABCD1234].mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).unwrap() {
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
        match classify(&p, Path::new("/a"), LibraryKind::Anime).unwrap() {
            Classification::Episode { episode, .. } => assert_eq!(episode, 12),
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_absolute_long_runner() {
        // One Piece-style absolute numbering past 1000.
        let p = PathBuf::from("/a/One Piece/One Piece - 1100.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).unwrap() {
            Classification::Episode { season, episode, .. } => {
                assert_eq!(season, 1);
                assert_eq!(episode, 1100);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_with_season_episode_tag_uses_show_parser() {
        // When the file does carry S01E05, prefer that over the absolute
        // path so users who organize anime by season aren't surprised.
        let p = PathBuf::from("/a/Attack on Titan/Season 4/Attack.on.Titan.S04E28.mkv");
        match classify(&p, Path::new("/a"), LibraryKind::Anime).unwrap() {
            Classification::Episode { season, episode, .. } => {
                assert_eq!(season, 4);
                assert_eq!(episode, 28);
            }
            _ => panic!("expected episode"),
        }
    }

    #[test]
    fn anime_rejects_no_episode_number() {
        let p = PathBuf::from("/a/Bleach/notes.mkv");
        assert!(classify(&p, Path::new("/a"), LibraryKind::Anime).is_none());
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
}
