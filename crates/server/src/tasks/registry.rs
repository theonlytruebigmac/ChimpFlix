//! The static kind registry. Adding a new task kind means appending
//! to [`REGISTRY`]; the gate logic, sweep dispatch, worker-pool
//! per-kind concurrency, and the admin API all pick it up.

#![allow(dead_code)] // `all_kinds` is the iteration surface for the upcoming admin API.

use std::sync::LazyLock;

use super::kind::{KindMetadata, TaskMode, TaskScope};

/// Every kind the binary knows how to run. Order is informational —
/// `find_kind` does a linear scan, but at <20 entries that's cheaper
/// than a hash lookup and keeps the source readable.
pub static REGISTRY: LazyLock<Vec<KindMetadata>> = LazyLock::new(|| {
    vec![
        // ─── Discovery pipeline · Automatic ───────────────────────
        KindMetadata {
            job_kind: "detect_markers_file",
            sweep_kind: Some("detect_markers"),
            display_name: "Marker detection",
            mode: TaskMode::Automatic,
            scope: TaskScope::PerFile,
            gate_setting_key: None,
            // CPU-bound: tacet runs symphonia decode + FFT per file.
            // Keep at 1 so it doesn't compete with live transcodes.
            concurrency: 1,
        },
        KindMetadata {
            job_kind: "generate_preview_sprite",
            sweep_kind: Some("generate_previews"),
            display_name: "Preview sprite",
            mode: TaskMode::Automatic,
            scope: TaskScope::PerFile,
            gate_setting_key: None,
            concurrency: 1, // ffmpeg sprite tile build
        },
        KindMetadata {
            job_kind: "refresh_logos_item",
            sweep_kind: Some("refresh_logos"),
            display_name: "Title-treatment logos",
            mode: TaskMode::Automatic,
            scope: TaskScope::PerItem,
            gate_setting_key: None,
            // Network-bound TMDB GET. 4 parallels a 1000-item
            // backfill nicely without thundering the rate limit
            // (TMDB allows 40 req/s).
            concurrency: 4,
        },
        KindMetadata {
            job_kind: "detect_extras_item",
            sweep_kind: Some("scan_extras"),
            display_name: "Extras & trailers",
            mode: TaskMode::Automatic,
            scope: TaskScope::PerItem,
            gate_setting_key: None,
            // Filesystem walk + per-file stat. 2 keeps a slow NAS
            // from being hammered but still parallels for typical
            // libraries.
            concurrency: 2,
        },
        // ─── Discovery pipeline · Gated ───────────────────────────
        KindMetadata {
            job_kind: "build_chapter_thumbs",
            sweep_kind: Some("generate_chapter_thumbs"),
            display_name: "Chapter thumbnails",
            mode: TaskMode::Gated,
            scope: TaskScope::PerFile,
            gate_setting_key: Some("chapter_thumbs_enabled"),
            concurrency: 1, // ffmpeg thumb extract
        },
        KindMetadata {
            job_kind: "analyze_loudness",
            sweep_kind: Some("analyze_loudness"),
            display_name: "Audio loudness analysis",
            mode: TaskMode::Gated,
            scope: TaskScope::PerFile,
            gate_setting_key: Some("loudness_analysis_enabled"),
            concurrency: 1, // ffmpeg loudnorm
        },
        KindMetadata {
            job_kind: "fetch_subtitles_item",
            sweep_kind: Some("fetch_subtitles"),
            display_name: "External subtitle fetch",
            mode: TaskMode::Gated,
            scope: TaskScope::PerItem,
            gate_setting_key: Some("subtitle_fetch_enabled"),
            concurrency: 4, // OpenSubtitles network-bound
        },
        KindMetadata {
            job_kind: "extract_embedded_subs",
            sweep_kind: Some("extract_subs_sweep"),
            display_name: "Embedded subtitle extract",
            mode: TaskMode::Gated,
            scope: TaskScope::PerFile,
            gate_setting_key: Some("embedded_subs_extract_enabled"),
            concurrency: 1, // ffmpeg + potential PGS OCR
        },
        KindMetadata {
            job_kind: "fetch_external_ratings",
            sweep_kind: Some("refresh_ratings"),
            display_name: "External ratings",
            mode: TaskMode::Gated,
            scope: TaskScope::PerItem,
            gate_setting_key: Some("external_ratings_enabled"),
            // Conservative against OMDb's 1000/day free-tier quota —
            // two in flight don't consume the budget in minutes.
            concurrency: 2,
        },
        // ─── Season-level (tacet bootstrap) ───────────────────────
        KindMetadata {
            job_kind: "bootstrap_season_refs",
            sweep_kind: None, // triggered by detect_markers_file, not on a cron
            display_name: "Season fingerprint bootstrap",
            mode: TaskMode::Automatic,
            scope: TaskScope::Global,
            gate_setting_key: None,
            // Heavy: decode + FFT every episode in a season inside
            // tacet's internal rayon pool. One outstanding bootstrap
            // at a time so two seasons don't dogpile CPU + disk.
            concurrency: 1,
        },
        // ─── Periodic system tasks (registry surface only — no job_queue rows) ───
        KindMetadata {
            // Periodic kinds have no job_kind in the queue; they
            // run directly from the scheduler. Mirror the sweep
            // name so the registry-driven gate check still
            // resolves them.
            job_kind: "rollup_task_metrics",
            sweep_kind: None,
            display_name: "Daily metrics rollup",
            mode: TaskMode::Periodic,
            scope: TaskScope::Global,
            gate_setting_key: None,
            // Runs inline on the scheduler tick; concurrency is
            // moot because the scheduler dispatcher only fires
            // each task once per due window. Set to 1 for
            // consistency with other Periodic entries.
            concurrency: 1,
        },
    ]
});

/// Look up a kind by either its job-side or sweep-side name. Returns
/// `None` for unknown kinds (custom operator-defined schedules, or
/// kinds added by a future binary that this older one doesn't know).
pub fn find_kind(name: &str) -> Option<&'static KindMetadata> {
    REGISTRY.iter().find(|k| k.matches(name))
}

/// All kinds the binary registers. Convenience accessor for callers
/// that want to iterate (admin UI, dependency-health probes).
pub fn all_kinds() -> &'static [KindMetadata] {
    REGISTRY.as_slice()
}

/// Returns the per-kind concurrency cap from the registry, or
/// `None` for kinds the registry doesn't know about. The worker
/// pool falls back to its own default (= total worker count) for
/// `None` so an unknown kind still runs without explicit setup.
pub fn concurrency_for(kind: &str) -> Option<u32> {
    find_kind(kind).map(|k| k.concurrency)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gated_kind_declares_a_setting_key() {
        for k in all_kinds() {
            match k.mode {
                TaskMode::Gated => assert!(
                    k.gate_setting_key.is_some(),
                    "gated kind {} must declare a gate_setting_key",
                    k.job_kind
                ),
                TaskMode::Automatic | TaskMode::Periodic => assert!(
                    k.gate_setting_key.is_none(),
                    "non-gated kind {} should not declare a gate_setting_key",
                    k.job_kind
                ),
            }
        }
    }

    #[test]
    fn every_kind_declares_concurrency() {
        for k in all_kinds() {
            assert!(
                k.concurrency >= 1,
                "kind {} declares concurrency 0 (use >= 1)",
                k.job_kind
            );
        }
    }

    #[test]
    fn job_kind_and_sweep_kind_resolve_to_same_metadata() {
        for k in all_kinds() {
            assert!(
                find_kind(k.job_kind).is_some(),
                "job_kind miss: {}",
                k.job_kind
            );
            if let Some(sweep) = k.sweep_kind {
                let by_sweep = find_kind(sweep).expect("sweep_kind miss");
                assert_eq!(by_sweep.job_kind, k.job_kind, "lookup divergence");
            }
        }
    }

    #[test]
    fn no_duplicate_job_kinds() {
        let mut seen = std::collections::HashSet::new();
        for k in all_kinds() {
            assert!(
                seen.insert(k.job_kind),
                "duplicate job_kind: {}",
                k.job_kind
            );
        }
    }

    #[test]
    fn unknown_kind_returns_none() {
        assert!(find_kind("not_a_real_kind").is_none());
    }

    #[test]
    fn concurrency_lookup_returns_registry_value() {
        assert_eq!(concurrency_for("detect_markers_file"), Some(1));
        assert_eq!(concurrency_for("refresh_logos_item"), Some(4));
        assert_eq!(concurrency_for("not_a_real_kind"), None);
    }
}
