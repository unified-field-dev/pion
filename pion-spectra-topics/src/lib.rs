//! Stable Spectra topic-name constants for the Pion control plane.
//!
//! Mirror of the `table:` / `name:` fields registered by `pion` under
//! [`spectra_core::SchemaRegistry`]. Depend on this crate when you need topic strings
//! (dashboards, NDJSON exporters, subscribers) without the full `pion` runtime stack.
//!
//! Wire naming: `spectra.event.{table}` and `spectra.metric.{name}` via
//! [`spectra_core::event_topic`] / [`spectra_core::metric_topic`].
//!
//! # Features
//!
//! - Event topics: [`event`] table names and `*_topic()` helpers.
//! - Metric topics: [`metric`] family names and `*_topic()` helpers.
//! - Store id: [`STORE`] (`"pion"`) shared by every registered schema.
//!
//! # Getting started
//!
//! Prerequisites: add `pion-spectra-topics` to your `Cargo.toml` (no Cargo feature flags).
//!
//! 1. Import [`event`] (or [`metric`]).
//! 2. Read a constant or call a `*_topic()` helper.
//! 3. Assert the wire string matches `spectra.event.*` / `spectra.metric.*`.
//!
//! ```
//! use pion_spectra_topics::event;
//!
//! assert_eq!(event::HEARTBEAT_LOG, "pion_heartbeat_log");
//! assert_eq!(event::heartbeat_log_topic(), "spectra.event.pion_heartbeat_log");
//! ```
//!
//! Variant: metric families use [`metric::heartbeats_ingested_topic`] →
//! `spectra.metric.pion_heartbeats_ingested`.
//!
//! # Keeping constants in sync
//!
//! Constants are hand-mirrored from `pion/pion/schemas/spectra/*_spectra_schema.rs` and
//! `*_spectra_metric.rs`. `tests/registry_sync.rs` links `pion` with `runtime` and asserts
//! every constant here matches a live [`spectra_core::SchemaRegistry`] entry (store +
//! [`spectra_core::LoggingKind`]), so drift fails CI.
//!
//! Until `spectra-codegen` ships topic helpers for this workspace, this crate stays
//! hand-written. `pion` does not depend on it today.

#![deny(missing_docs)]

/// Logical Spectra `store` name every `pion` schema/metric registers under.
///
/// Mirrors the `store: "pion"` field shared by every `spectra_schema!` / `spectra_metric!`
/// declaration in `pion/pion/schemas/spectra/`.
pub const STORE: &str = "pion";

/// Event (structured log table) topic names and topic-string helpers.
pub mod event {
    /// `pion_heartbeat_log` — Parton heartbeat ingest and container transition trace.
    pub const HEARTBEAT_LOG: &str = "pion_heartbeat_log";
    /// `pion_handoff_log` — Handoff directive deliver, ack, and collect trace.
    pub const HANDOFF_LOG: &str = "pion_handoff_log";
    /// `pion_migration_log` — progress of the legacy control-plane table migration.
    pub const MIGRATION_LOG: &str = "pion_migration_log";
    /// `pion_node_action_log` — Control-plane node action queue lifecycle trace.
    pub const NODE_ACTION_LOG: &str = "pion_node_action_log";
    /// `pion_server_boot_log` — `pion-server` bootstrap failures and operator drill-down.
    pub const SERVER_BOOT_LOG: &str = "pion_server_boot_log";

    /// All registered `pion` event table names, in declaration order.
    pub const ALL: &[&str] = &[
        HEARTBEAT_LOG,
        HANDOFF_LOG,
        MIGRATION_LOG,
        NODE_ACTION_LOG,
        SERVER_BOOT_LOG,
    ];

    /// Wire-level Spectra topic for [`HEARTBEAT_LOG`] (`spectra.event.pion_heartbeat_log`).
    #[must_use]
    pub fn heartbeat_log_topic() -> String {
        spectra_core::event_topic(HEARTBEAT_LOG)
    }

    /// Wire-level Spectra topic for [`HANDOFF_LOG`] (`spectra.event.pion_handoff_log`).
    #[must_use]
    pub fn handoff_log_topic() -> String {
        spectra_core::event_topic(HANDOFF_LOG)
    }

    /// Wire-level Spectra topic for [`MIGRATION_LOG`] (`spectra.event.pion_migration_log`).
    #[must_use]
    pub fn migration_log_topic() -> String {
        spectra_core::event_topic(MIGRATION_LOG)
    }

    /// Wire-level Spectra topic for [`NODE_ACTION_LOG`] (`spectra.event.pion_node_action_log`).
    #[must_use]
    pub fn node_action_log_topic() -> String {
        spectra_core::event_topic(NODE_ACTION_LOG)
    }

    /// Wire-level Spectra topic for [`SERVER_BOOT_LOG`] (`spectra.event.pion_server_boot_log`).
    #[must_use]
    pub fn server_boot_log_topic() -> String {
        spectra_core::event_topic(SERVER_BOOT_LOG)
    }
}

/// Metric (counter/gauge family) topic names and topic-string helpers.
pub mod metric {
    /// `pion_handoff_directives` — Handoff directive deliver/ack/collect outcomes.
    pub const HANDOFF_DIRECTIVES: &str = "pion_handoff_directives";
    /// `pion_heartbeat_publishes` — Container state Photon publish outcomes.
    pub const HEARTBEAT_PUBLISHES: &str = "pion_heartbeat_publishes";
    /// `pion_heartbeats_ingested` — Parton heartbeat ingest outcomes.
    pub const HEARTBEATS_INGESTED: &str = "pion_heartbeats_ingested";
    /// `pion_migration_runs` — One-off migration script table operations.
    pub const MIGRATION_RUNS: &str = "pion_migration_runs";
    /// `pion_node_actions` — Control-plane node action queue outcomes.
    pub const NODE_ACTIONS: &str = "pion_node_actions";
    /// `pion_server_boot_steps` — `pion-server` bootstrap step outcomes.
    pub const SERVER_BOOT_STEPS: &str = "pion_server_boot_steps";

    /// All registered `pion` metric family names, in declaration order.
    pub const ALL: &[&str] = &[
        HANDOFF_DIRECTIVES,
        HEARTBEAT_PUBLISHES,
        HEARTBEATS_INGESTED,
        MIGRATION_RUNS,
        NODE_ACTIONS,
        SERVER_BOOT_STEPS,
    ];

    /// Wire-level Spectra topic for [`HANDOFF_DIRECTIVES`] (`spectra.metric.pion_handoff_directives`).
    #[must_use]
    pub fn handoff_directives_topic() -> String {
        spectra_core::metric_topic(HANDOFF_DIRECTIVES)
    }

    /// Wire-level Spectra topic for [`HEARTBEAT_PUBLISHES`] (`spectra.metric.pion_heartbeat_publishes`).
    #[must_use]
    pub fn heartbeat_publishes_topic() -> String {
        spectra_core::metric_topic(HEARTBEAT_PUBLISHES)
    }

    /// Wire-level Spectra topic for [`HEARTBEATS_INGESTED`] (`spectra.metric.pion_heartbeats_ingested`).
    #[must_use]
    pub fn heartbeats_ingested_topic() -> String {
        spectra_core::metric_topic(HEARTBEATS_INGESTED)
    }

    /// Wire-level Spectra topic for [`MIGRATION_RUNS`] (`spectra.metric.pion_migration_runs`).
    #[must_use]
    pub fn migration_runs_topic() -> String {
        spectra_core::metric_topic(MIGRATION_RUNS)
    }

    /// Wire-level Spectra topic for [`NODE_ACTIONS`] (`spectra.metric.pion_node_actions`).
    #[must_use]
    pub fn node_actions_topic() -> String {
        spectra_core::metric_topic(NODE_ACTIONS)
    }

    /// Wire-level Spectra topic for [`SERVER_BOOT_STEPS`] (`spectra.metric.pion_server_boot_steps`).
    #[must_use]
    pub fn server_boot_steps_topic() -> String {
        spectra_core::metric_topic(SERVER_BOOT_STEPS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_topics_follow_spectra_event_convention() {
        for &name in event::ALL {
            assert_eq!(
                spectra_core::event_topic(name),
                format!("spectra.event.{name}")
            );
        }
        assert_eq!(
            event::heartbeat_log_topic(),
            "spectra.event.pion_heartbeat_log"
        );
    }

    #[test]
    fn metric_topics_follow_spectra_metric_convention() {
        for &name in metric::ALL {
            assert_eq!(
                spectra_core::metric_topic(name),
                format!("spectra.metric.{name}")
            );
        }
        assert_eq!(
            metric::handoff_directives_topic(),
            "spectra.metric.pion_handoff_directives"
        );
    }

    #[test]
    fn event_and_metric_names_are_distinct_and_non_empty() {
        let mut all: Vec<&str> = event::ALL
            .iter()
            .copied()
            .chain(metric::ALL.iter().copied())
            .collect();
        assert!(all.iter().all(|name| !name.is_empty()));
        let before = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(
            all.len(),
            before,
            "duplicate topic name across event/metric sets"
        );
    }
}
