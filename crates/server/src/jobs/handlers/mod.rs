//! Per-kind job handlers. New kinds are added as submodules here and
//! registered in [`super::build_router`].

pub mod analyze_loudness;
pub mod build_chapter_thumbs;
pub mod detect_markers_file;
pub mod fetch_subtitles_item;
pub mod generate_preview_sprite;
