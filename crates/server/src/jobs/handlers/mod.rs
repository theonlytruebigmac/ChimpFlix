//! Per-kind job handlers. New kinds are added as submodules here and
//! registered in [`super::build_router`].

pub mod analyze_loudness;
pub mod bootstrap_season_refs;
pub mod build_chapter_thumbs;
pub mod detect_extras_item;
pub mod detect_markers_file;
pub mod extract_embedded_subs;
pub mod fetch_external_ratings;
pub mod fetch_subtitles_item;
pub mod generate_preview_sprite;
pub mod refresh_logos_item;
