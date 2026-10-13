//! Pion product-owned Spectra telemetry (metrics + events).
//!
//! Emit via [`spectra_core`] public crate; host binaries persist `pion_*` tables through
//! [`pion_spectra_topics`](https://github.com/unified-field-dev/pion/blob/main/pion-spectra-topics) sink forwarding.

mod events;

pub mod boot;
pub mod handoff;
pub mod heartbeat;
pub mod migration;
pub mod node_action;
pub mod sink;

pub use boot::{log_boot_step, warn_boot_step};
pub use handoff::HandoffDirectiveContext;
pub use heartbeat::HeartbeatContext;
pub use node_action::{enqueue_noop_message, terminal_failure_message, NodeActionContext};
