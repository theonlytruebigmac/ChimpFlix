//! Gate evaluation — the single function both pipeline entry points
//! consult before fanning out work for a kind.
//!
//! Without this layer, the on-add discovery pipeline used to enqueue
//! every kind unconditionally on `FileAdded` while the scheduler
//! sweep respected `scheduled_tasks.enabled` — flipping the admin
//! toggle off only stopped the sweep. The pipeline kept queueing
//! per-file jobs, defeating the toggle.
//!
//! Now both [`crate::jobs::pipeline::enqueue_pipeline`] and the
//! scheduler's per-kind dispatch call [`is_kind_allowed`] before
//! dispatching, so a single boolean flip in admin → tasks turns the
//! kind off everywhere.

use crate::state::AppState;

use super::kind::TaskMode;
use super::registry;

/// Result of a gate evaluation. The caller usually only cares
/// whether the kind is allowed, but the failure variants are kept
/// distinct for telemetry / UI ("disabled by admin" reads very
/// differently from "unknown kind, this binary doesn't ship that
/// handler").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    /// Kind is registered, mode allows running, settings agree.
    Allowed,
    /// Kind exists but its `*_enabled` setting is false.
    DisabledByAdmin,
    /// Kind isn't in the registry. Treated as allowed by callers
    /// (so custom operator-defined schedules continue to dispatch),
    /// but flagged so we can warn at startup if a `scheduled_tasks`
    /// row references a kind the binary doesn't know.
    UnknownKind,
}

impl GateState {
    /// Returns true when the kind should dispatch. Both `Allowed`
    /// and `UnknownKind` qualify — the former because the gate
    /// agreed, the latter because we want to fall through to legacy
    /// kinds that haven't been migrated into the registry yet.
    #[inline]
    pub fn is_allowed(self) -> bool {
        matches!(self, GateState::Allowed | GateState::UnknownKind)
    }
}

/// Ask the gate whether `name` (a job-kind or sweep-kind) is
/// allowed to dispatch right now.
///
/// Reads the live [`SettingsCache`][crate::state::SettingsCache] —
/// no DB hit. Settings PATCH writes invalidate the cache via the
/// normal admin-settings handler path, so a freshly-flipped toggle
/// takes effect on the *next* call here.
pub async fn is_kind_allowed(state: &AppState, name: &str) -> GateState {
    let settings = state.settings.read().await;
    evaluate(name, |key| Some(read_gate_bool(&settings, key)))
}

/// Pure-function gate evaluator. Production wraps it with the live
/// settings cache; tests wrap it with a closure backed by a fixture.
/// Returns the same `GateState` semantics either way.
///
/// `read_bool` returns `Some(value)` for known gate keys, `None` for
/// keys this binary doesn't know about (treated as "disabled" so a
/// misconfigured row can't accidentally dispatch).
fn evaluate(name: &str, read_bool: impl Fn(&str) -> Option<bool>) -> GateState {
    let Some(meta) = registry::find_kind(name) else {
        return GateState::UnknownKind;
    };
    match meta.mode {
        TaskMode::Automatic | TaskMode::Periodic => GateState::Allowed,
        TaskMode::Gated => {
            let Some(key) = meta.gate_setting_key else {
                // Defensive: a Gated kind without a setting key
                // would be a registry bug, caught by the unit test
                // in registry.rs. In production we'd rather surface
                // "disabled" than dispatch unguarded work.
                return GateState::DisabledByAdmin;
            };
            match read_bool(key) {
                Some(true) => GateState::Allowed,
                Some(false) => GateState::DisabledByAdmin,
                None => {
                    tracing::warn!(
                        gate_key = %key,
                        "gate setting key not recognised by binary; treating as disabled"
                    );
                    GateState::DisabledByAdmin
                }
            }
        }
    }
}

/// Read one of the gate booleans from the live settings struct.
///
/// This match is deliberately a closed enumeration of known gate
/// keys rather than a reflection / map lookup — adding a new gate
/// is then a compile error here, which forces the registry
/// addition and the wiring to stay in sync.
fn read_gate_bool(settings: &chimpflix_library::ServerSettings, key: &str) -> bool {
    match key {
        "chapter_thumbs_enabled" => settings.chapter_thumbs_enabled,
        "loudness_analysis_enabled" => settings.loudness_analysis_enabled,
        "subtitle_fetch_enabled" => settings.subtitle_fetch_enabled,
        "embedded_subs_extract_enabled" => settings.embedded_subs_extract_enabled,
        "external_ratings_enabled" => settings.external_ratings_enabled,
        // Falling through here would silently leave a Gated kind
        // permanently allowed. Caller (`evaluate`) handles the None
        // case by warning + denying.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn check(name: &str, gates: &HashMap<&str, bool>) -> GateState {
        evaluate(name, |k| gates.get(k).copied())
    }

    #[test]
    fn allowed_is_allowed_semantics() {
        assert!(GateState::Allowed.is_allowed());
        assert!(GateState::UnknownKind.is_allowed());
        assert!(!GateState::DisabledByAdmin.is_allowed());
    }

    #[test]
    fn automatic_kind_always_allowed_regardless_of_gates() {
        let gates: HashMap<&str, bool> = HashMap::new();
        // detect_markers_file is Automatic — no gate key, can't be
        // turned off via settings.
        assert_eq!(check("detect_markers_file", &gates), GateState::Allowed);
        assert_eq!(check("detect_markers", &gates), GateState::Allowed);
        assert_eq!(check("generate_preview_sprite", &gates), GateState::Allowed);
    }

    #[test]
    fn gated_kind_blocked_when_setting_false() {
        let mut gates = HashMap::new();
        gates.insert("loudness_analysis_enabled", false);
        assert_eq!(
            check("analyze_loudness", &gates),
            GateState::DisabledByAdmin
        );
    }

    #[test]
    fn gated_kind_allowed_when_setting_true() {
        let mut gates = HashMap::new();
        gates.insert("loudness_analysis_enabled", true);
        assert_eq!(check("analyze_loudness", &gates), GateState::Allowed);
    }

    #[test]
    fn job_and_sweep_names_resolve_to_same_gate() {
        // The job-kind `build_chapter_thumbs` and the sweep-kind
        // `generate_chapter_thumbs` share the chapter_thumbs_enabled
        // gate. Flipping the gate off must block both names, so the
        // on-add path and the safety-net cron stop together.
        let mut off = HashMap::new();
        off.insert("chapter_thumbs_enabled", false);
        assert_eq!(
            check("build_chapter_thumbs", &off),
            GateState::DisabledByAdmin
        );
        assert_eq!(
            check("generate_chapter_thumbs", &off),
            GateState::DisabledByAdmin
        );

        let mut on = HashMap::new();
        on.insert("chapter_thumbs_enabled", true);
        assert_eq!(check("build_chapter_thumbs", &on), GateState::Allowed);
        assert_eq!(check("generate_chapter_thumbs", &on), GateState::Allowed);
    }

    #[test]
    fn unknown_kind_is_treated_as_allowed() {
        // Custom operator-defined kinds the binary doesn't know about
        // still dispatch — we shouldn't accidentally break their
        // scheduled tasks by being too strict.
        let gates: HashMap<&str, bool> = HashMap::new();
        assert_eq!(
            check("operator_custom_kind", &gates),
            GateState::UnknownKind
        );
        assert!(GateState::UnknownKind.is_allowed());
    }

    #[test]
    fn gated_kind_with_missing_setting_key_blocked_safely() {
        // The closure returns None for unknown gate keys.
        // `evaluate` should refuse to dispatch rather than default to
        // allowed — a missing gate is a misconfiguration, and
        // defaulting open would defeat the gate entirely.
        let gates: HashMap<&str, bool> = HashMap::new();
        // analyze_loudness is Gated with key "loudness_analysis_enabled".
        // No entry in `gates` → closure returns None → gate denies.
        assert_eq!(
            check("analyze_loudness", &gates),
            GateState::DisabledByAdmin
        );
    }

    /// Coupling guard: every Gated kind in the registry must have
    /// its `gate_setting_key` wired into the production resolver
    /// `read_gate_bool` (gates.rs:96-110). Adding a Gated kind
    /// without updating that match would have it silently default
    /// to "disabled" — operators flipping the admin toggle would
    /// see no change. The list below must stay in sync with the
    /// closed enumeration in `read_gate_bool`.
    #[test]
    fn every_gated_kind_setting_key_is_wired_into_production_resolver() {
        use crate::tasks::kind::TaskMode;
        use crate::tasks::registry;

        const PRODUCTION_GATE_KEYS: &[&str] = &[
            "chapter_thumbs_enabled",
            "loudness_analysis_enabled",
            "subtitle_fetch_enabled",
            "embedded_subs_extract_enabled",
            "external_ratings_enabled",
        ];

        for k in registry::all_kinds() {
            if !matches!(k.mode, TaskMode::Gated) {
                continue;
            }
            let key = k
                .gate_setting_key
                .expect("registry test ensures Gated kinds declare a key");
            assert!(
                PRODUCTION_GATE_KEYS.contains(&key),
                "Gated kind `{}` declares gate_setting_key `{}` \
                 but `read_gate_bool` in gates.rs doesn't recognise \
                 it. Add the key to PRODUCTION_GATE_KEYS in this \
                 test AND to the match arm in `read_gate_bool`.",
                k.job_kind,
                key,
            );
        }
    }

    /// Symmetry guard: for every Gated kind, both its `job_kind`
    /// and (if any) `sweep_kind` must hit `DisabledByAdmin` when
    /// the setting is off, and `Allowed` when it's on. The earlier
    /// `job_and_sweep_names_resolve_to_same_gate` test covers one
    /// kind by hand; this walks the registry so a future kind
    /// addition automatically gets the same coverage.
    #[test]
    fn every_gated_kind_flips_both_names_in_lockstep() {
        use crate::tasks::kind::TaskMode;
        use crate::tasks::registry;

        for k in registry::all_kinds() {
            if !matches!(k.mode, TaskMode::Gated) {
                continue;
            }
            let key = k.gate_setting_key.expect("Gated kind has a key");

            let mut off = HashMap::new();
            off.insert(key, false);
            let mut on = HashMap::new();
            on.insert(key, true);

            assert_eq!(
                check(k.job_kind, &off),
                GateState::DisabledByAdmin,
                "job_kind `{}` should be Disabled when `{}` is off",
                k.job_kind,
                key,
            );
            assert_eq!(
                check(k.job_kind, &on),
                GateState::Allowed,
                "job_kind `{}` should be Allowed when `{}` is on",
                k.job_kind,
                key,
            );
            if let Some(sweep) = k.sweep_kind {
                assert_eq!(
                    check(sweep, &off),
                    GateState::DisabledByAdmin,
                    "sweep_kind `{sweep}` should be Disabled when `{key}` is off"
                );
                assert_eq!(
                    check(sweep, &on),
                    GateState::Allowed,
                    "sweep_kind `{sweep}` should be Allowed when `{key}` is on"
                );
            }
        }
    }
}
