//! Declarative scenario steps shared by e2e (assert) and bench (measure).

use serde::{Deserialize, Serialize};

/// One step in an agent / control-plane scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum ScenarioStep {
    /// Create a host enrollment ticket and remember the wire token.
    EnrollAgent,
    /// Heartbeat until the node is accepted (stub: one successful ingest).
    HeartbeatUntilHealthy,
    /// `docker pull` the shared bench image (warm path; not counted in cold TTR).
    PrePullImage,
    /// Enqueue a `deploy_handoff` node action for the harness node.
    EnqueueDeploy,
    /// Claim the pending action and acknowledge without executing (schedule latency).
    ClaimAndAck,
    /// Claim the pending action and execute it (stub or docker-cli).
    ClaimAndExecute,
    /// Assert the target container is running (stub state or observation).
    AssertContainerRunning,
    /// Enqueue a `teardown_handoff` node action.
    EnqueueTeardown,
    /// Assert the target container is gone.
    AssertContainerGone,
    /// Concurrent deploy firehose (N actions, bounded concurrency).
    FirehoseDeploy {
        /// Number of deploy actions.
        n: u32,
        /// Max in-flight deploys.
        concurrency: u32,
    },
    /// Concurrent teardown firehose.
    FirehoseTeardown {
        /// Number of teardown actions.
        n: u32,
        /// Max in-flight teardowns.
        concurrency: u32,
    },
    /// Extend the lease on the current claimed command (stub OK).
    ExtendLease,
    /// Cancel a pending/running action (stub OK).
    CancelAction,
    /// Apply a `ReEnroll` handoff directive (stub OK).
    ApplyReEnroll,
    /// Apply a Revoke handoff directive (stub OK).
    ApplyRevoke,
}

/// Named scenario: stable id + ordered steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioSpec {
    /// Stable scenario identifier.
    pub id: String,
    /// Ordered steps to execute.
    pub steps: Vec<ScenarioStep>,
}

impl ScenarioSpec {
    /// Enroll → heartbeat → `deploy_handoff` → claim → stub execute → success.
    ///
    /// Primary in-process correctness smoke used by `pion-e2e` and the single-deploy benchmark warm-up.
    #[must_use]
    pub fn enroll_deploy_smoke() -> Self {
        Self {
            id: "enroll-deploy-smoke".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::EnqueueDeploy,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerRunning,
            ],
        }
    }

    /// `LocalDocker` / AWS warm deploy with pre-pull (warm deploy benchmark and the Docker-gated e2e test).
    #[must_use]
    pub fn enroll_deploy_warm_docker() -> Self {
        Self {
            id: "enroll-deploy-warm-docker".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::PrePullImage,
                ScenarioStep::EnqueueDeploy,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerRunning,
            ],
        }
    }

    /// Warm deploy + teardown → gone (teardown benchmark).
    #[must_use]
    pub fn enroll_teardown_gone_docker() -> Self {
        Self {
            id: "enroll-teardown-gone-docker".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::PrePullImage,
                ScenarioStep::EnqueueDeploy,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerRunning,
                ScenarioStep::EnqueueTeardown,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerGone,
            ],
        }
    }

    /// Cold pull + deploy (cold-start benchmark): no `PrePullImage`; pull happens inside deploy if missing.
    #[must_use]
    pub fn enroll_deploy_cold_docker() -> Self {
        Self {
            id: "enroll-deploy-cold-docker".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::EnqueueDeploy,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerRunning,
            ],
        }
    }

    /// Schedule path only: enqueue → claim ack (scheduling benchmark).
    #[must_use]
    pub fn schedule_claim_smoke() -> Self {
        Self {
            id: "schedule-claim-smoke".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::EnqueueDeploy,
                ScenarioStep::ClaimAndAck,
            ],
        }
    }

    /// Heartbeat round-trip smoke: enroll once, then repeated healthy heartbeats.
    #[must_use]
    pub fn heartbeat_rtt_smoke() -> Self {
        Self {
            id: "heartbeat-rtt-smoke".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
            ],
        }
    }

    /// Single teardown after a warm stub deploy (stub teardown benchmark).
    #[must_use]
    pub fn enroll_teardown_smoke() -> Self {
        Self {
            id: "enroll-teardown-smoke".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::EnqueueDeploy,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerRunning,
                ScenarioStep::EnqueueTeardown,
                ScenarioStep::ClaimAndExecute,
                ScenarioStep::AssertContainerGone,
            ],
        }
    }

    /// Small firehose stub: N=10, c=2.
    #[must_use]
    pub fn firehose_deploy_stub() -> Self {
        Self {
            id: "firehose-deploy-stub".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::FirehoseDeploy {
                    n: 10,
                    concurrency: 2,
                },
            ],
        }
    }

    /// Edge firehose: N=50, c=4 with optional pre-pull for warm starts.
    #[must_use]
    pub fn firehose_deploy_df0() -> Self {
        Self {
            id: "firehose-deploy-df0".into(),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::PrePullImage,
                ScenarioStep::FirehoseDeploy {
                    n: 50,
                    concurrency: 4,
                },
            ],
        }
    }

    /// Configurable firehose (custom N and concurrency, multi-agent cells).
    #[must_use]
    pub fn firehose_deploy_n(n: u32, concurrency: u32) -> Self {
        Self {
            id: format!("firehose-deploy-n{n}-c{concurrency}"),
            steps: vec![
                ScenarioStep::EnrollAgent,
                ScenarioStep::HeartbeatUntilHealthy,
                ScenarioStep::PrePullImage,
                ScenarioStep::FirehoseDeploy { n, concurrency },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_deploy_smoke_roundtrips_json() {
        let spec = ScenarioSpec::enroll_deploy_smoke();
        let json = serde_json::to_string(&spec).expect("serialize");
        let back: ScenarioSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(spec, back);
    }
}
