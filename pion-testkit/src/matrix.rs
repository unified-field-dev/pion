//! Benchmark/e2e dimension matrix for Pion + Parton latency and correctness runs.

use serde::{Deserialize, Serialize};

/// Where the control plane and agents run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Topology {
    /// In-process CP + stub/fake Docker (no network, no live containers).
    #[default]
    #[serde(alias = "isolated-lab")]
    IsolatedHarness,
    /// Single host with a real Docker daemon.
    LocalDocker,
    /// AWS same-region CP + agents (decision-grade same-AZ baselines).
    AwsSameRegion,
    /// AWS cross-region: CP in region A, agents in region B.
    AwsWan,
}

impl Topology {
    /// Stable CLI / report string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IsolatedHarness => "isolated-harness",
            Self::LocalDocker => "local-docker",
            Self::AwsSameRegion => "aws-same-region",
            Self::AwsWan => "aws-wan",
        }
    }
}

impl std::str::FromStr for Topology {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "isolated-harness" | "isolated-lab" => Ok(Self::IsolatedHarness),
            "local-docker" => Ok(Self::LocalDocker),
            "aws-same-region" => Ok(Self::AwsSameRegion),
            "aws-wan" => Ok(Self::AwsWan),
            _ => Err(()),
        }
    }
}

/// How container actions are executed on the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Executor {
    /// In-process stub — no Docker CLI, records synthetic container state.
    #[default]
    Stub,
    /// Shell out to the host Docker CLI.
    DockerCli,
}

impl Executor {
    /// Stable CLI / report string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stub => "stub",
            Self::DockerCli => "docker-cli",
        }
    }
}

/// Valence storage backend for the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    /// `SQLite` (embedded / smoke).
    #[default]
    Sqlite,
    /// Hybrid Postgres + in-process `IndraDB` (primary).
    Hybrid,
}

impl Backend {
    /// Stable CLI / report string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Hybrid => "hybrid",
        }
    }
}

/// Telemetry adapter for the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TelemetryAdapter {
    /// No ops log / Spectra sink.
    #[default]
    Off,
    /// Console ops log.
    Console,
}

impl TelemetryAdapter {
    /// Stable CLI / report string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Console => "console",
        }
    }
}

/// Hardware profile label for reports (not a live provisioner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Hardware {
    /// Local workstation; non-authoritative, for development only.
    #[default]
    DevWsl,
    /// AWS `t3.large` / medium-class smoke.
    AwsT3Large,
    /// AWS `c6i.large` decision-grade primary row.
    AwsC6iLarge,
}

impl Hardware {
    /// Stable CLI / report string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DevWsl => "dev-wsl",
            Self::AwsT3Large => "aws-t3-large",
            Self::AwsC6iLarge => "aws-c6i-large",
        }
    }

    /// Whether this label is allowed in authoritative benchmark reports.
    #[must_use]
    pub const fn is_authoritative(self) -> bool {
        matches!(self, Self::AwsT3Large | Self::AwsC6iLarge)
    }
}

/// Full cross-product selector for e2e and bench drivers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixSpec {
    /// Runtime topology.
    pub topology: Topology,
    /// Container executor.
    pub executor: Executor,
    /// Valence storage backend.
    pub backend: Backend,
    /// Telemetry adapter.
    pub telemetry: TelemetryAdapter,
    /// Hardware profile label.
    pub hardware: Hardware,
}

impl Default for MatrixSpec {
    fn default() -> Self {
        Self::isolated_harness_stub()
    }
}

impl MatrixSpec {
    /// Default CI / local smoke row: in-process CP + stub executor + `SQLite`.
    #[must_use]
    pub const fn isolated_harness_stub() -> Self {
        Self {
            topology: Topology::IsolatedHarness,
            executor: Executor::Stub,
            backend: Backend::Sqlite,
            telemetry: TelemetryAdapter::Off,
            hardware: Hardware::DevWsl,
        }
    }

    /// Local Docker row (gated behind `PION_E2E_DOCKER=1` in e2e).
    #[must_use]
    pub const fn local_docker() -> Self {
        Self {
            topology: Topology::LocalDocker,
            executor: Executor::DockerCli,
            backend: Backend::Sqlite,
            telemetry: TelemetryAdapter::Off,
            hardware: Hardware::DevWsl,
        }
    }

    /// In-process harness stub tagged with a hardware label (for example a same-region EC2 smoke run).
    #[must_use]
    pub const fn isolated_harness_stub_on(hardware: Hardware) -> Self {
        Self {
            topology: Topology::IsolatedHarness,
            executor: Executor::Stub,
            backend: Backend::Sqlite,
            telemetry: TelemetryAdapter::Off,
            hardware,
        }
    }

    /// AWS same-region hybrid primary row (docker-cli).
    #[must_use]
    pub const fn aws_same_region_hybrid(hardware: Hardware) -> Self {
        Self {
            topology: Topology::AwsSameRegion,
            executor: Executor::DockerCli,
            backend: Backend::Hybrid,
            telemetry: TelemetryAdapter::Off,
            hardware,
        }
    }

    /// AWS same-region `SQLite` smoke row (docker-cli).
    #[must_use]
    pub const fn aws_same_region_sqlite(hardware: Hardware) -> Self {
        Self {
            topology: Topology::AwsSameRegion,
            executor: Executor::DockerCli,
            backend: Backend::Sqlite,
            telemetry: TelemetryAdapter::Off,
            hardware,
        }
    }

    /// AWS cross-region (WAN) hybrid primary row (docker-cli on the remote agent host).
    #[must_use]
    pub const fn aws_wan_hybrid(hardware: Hardware) -> Self {
        Self {
            topology: Topology::AwsWan,
            executor: Executor::DockerCli,
            backend: Backend::Hybrid,
            telemetry: TelemetryAdapter::Off,
            hardware,
        }
    }

    /// Override backend on a copy of this matrix.
    #[must_use]
    pub const fn with_backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }

    /// Override hardware on a copy of this matrix.
    #[must_use]
    pub const fn with_hardware(mut self, hardware: Hardware) -> Self {
        self.hardware = hardware;
        self
    }

    /// Stable string id for report filenames.
    #[must_use]
    pub fn report_slug(&self) -> String {
        format!(
            "{}-{}-{}-{}-{}",
            self.topology.as_str(),
            self.executor.as_str(),
            self.backend.as_str(),
            self.telemetry.as_str(),
            self.hardware.as_str(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_harness_stub_defaults() {
        let m = MatrixSpec::isolated_harness_stub();
        assert_eq!(m.topology, Topology::IsolatedHarness);
        assert_eq!(m.executor, Executor::Stub);
        assert_eq!(m.backend, Backend::Sqlite);
        assert!(m.report_slug().contains("sqlite"));
    }

    #[test]
    fn aws_wan_hybrid_uses_docker_cli_and_wan_topology() {
        let m = MatrixSpec::aws_wan_hybrid(Hardware::AwsC6iLarge);
        assert_eq!(m.topology, Topology::AwsWan);
        assert_eq!(m.executor, Executor::DockerCli);
        assert_eq!(m.backend, Backend::Hybrid);
        assert_eq!(Topology::AwsWan.as_str(), "aws-wan");
        assert!(m.report_slug().starts_with("aws-wan-docker-cli-hybrid-"));
    }
}
