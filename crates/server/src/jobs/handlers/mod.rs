//! Per-kind job handlers. New kinds are added as submodules here and
//! registered in [`super::build_router`].

pub mod analyze_loudness;
pub mod bootstrap_season_refs;
pub mod detect_extras_item;
pub mod detect_markers_file;
pub mod extract_embedded_subs;
pub mod fetch_external_ratings;
pub mod fetch_subtitles_item;
pub mod notify_new_content;
pub mod refresh_logos_item;
pub mod trakt_pull_user;
pub mod trakt_push_user_history;
