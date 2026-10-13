//! Pion + Parton benchmarks: heartbeat round trip, deploy, teardown and firehose scenarios.
//!
//! Results are authoritative only for AWS hardware labels. Measured numbers and guidance are in
//! [`PERFORMANCE.md`](https://github.com/unified-field-dev/pion/blob/main/pion-bench/PERFORMANCE.md).
//! List and run experiments with this CLI.
//!
//! # Getting started
//!
//! ```bash
//! cargo run -p pion-bench -- experiments
//! cargo run -p pion-bench -- run --experiment bm-r0
//! ```
//!
//! Observable outcome: `experiments` prints registered ids; `run` writes a JSON report
//! (path printed / configured via CLI flags). Harness: `pion-testkit`.

mod stats;

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};
use pion_testkit::{
    bench_image_ref, resolve_image_digest, Backend, BootstrapSession, Executor, Hardware,
    MatrixSpec, RunMode, ScenarioRunner, ScenarioSpec, Topology,
};
use serde::Serialize;
use stats::MetricStats;

/// Pion + Parton benchmark CLI.
#[derive(Debug, Parser)]
#[command(
    name = "pion-bench",
    about = "Pion/Parton benchmarks (AWS runs are authoritative; deploys use a shared nginx image)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List registered experiment IDs.
    Experiments,
    /// Run one experiment and write a JSON report.
    Run {
        /// Experiment id (`bm-d1`, `bm-r0`, …).
        #[arg(long, value_enum)]
        experiment: ExperimentId,
        /// Optional report path.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Iterations / sample count for latency rows.
        #[arg(long, default_value_t = 20)]
        ops: u32,
        /// Hardware label (authoritative: aws-c6i-large / aws-t3-large).
        #[arg(long, value_enum, default_value_t = HardwareArg::AwsC6iLarge)]
        hardware: HardwareArg,
        /// Valence backend (`sqlite` smoke or `hybrid` primary).
        #[arg(long, value_enum, default_value_t = BackendArg::Sqlite)]
        backend: BackendArg,
        /// Topology for the matrix slug.
        #[arg(long, value_enum, default_value_t = TopologyArg::AwsSameRegion)]
        topology: TopologyArg,
        /// Executor (`stub` or `docker-cli`).
        #[arg(long, value_enum, default_value_t = ExecutorArg::Stub)]
        executor: ExecutorArg,
        /// Firehose N (used by the `bm-df1` experiment).
        #[arg(long)]
        firehose_n: Option<u32>,
        /// Firehose concurrency.
        #[arg(long)]
        firehose_c: Option<u32>,
        /// Agent count label for multi-agent efficiency rows (`bm-dm1`).
        #[arg(long, default_value_t = 1)]
        agent_count: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HardwareArg {
    #[value(name = "dev-wsl")]
    DevWsl,
    #[value(name = "aws-t3-large")]
    AwsT3Large,
    #[value(name = "aws-c6i-large")]
    AwsC6iLarge,
}

impl HardwareArg {
    const fn to_hardware(self) -> Hardware {
        match self {
            Self::DevWsl => Hardware::DevWsl,
            Self::AwsT3Large => Hardware::AwsT3Large,
            Self::AwsC6iLarge => Hardware::AwsC6iLarge,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BackendArg {
    #[value(name = "sqlite")]
    Sqlite,
    #[value(name = "hybrid")]
    Hybrid,
}

impl BackendArg {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Sqlite => Backend::Sqlite,
            Self::Hybrid => Backend::Hybrid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TopologyArg {
    #[value(name = "isolated-harness")]
    #[value(name = "isolated-lab")]
    IsolatedHarness,
    #[value(name = "local-docker")]
    LocalDocker,
    #[value(name = "aws-same-region")]
    AwsSameRegion,
    /// Cross-region: control plane in region A, agent host in region B.
    #[value(name = "aws-wan")]
    AwsWan,
}

impl TopologyArg {
    const fn to_topology(self) -> Topology {
        match self {
            Self::IsolatedHarness => Topology::IsolatedHarness,
            Self::LocalDocker => Topology::LocalDocker,
            Self::AwsSameRegion => Topology::AwsSameRegion,
            Self::AwsWan => Topology::AwsWan,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ExecutorArg {
    #[value(name = "stub")]
    Stub,
    #[value(name = "docker-cli")]
    DockerCli,
}

impl ExecutorArg {
    const fn to_executor(self) -> Executor {
        match self {
            Self::Stub => Executor::Stub,
            Self::DockerCli => Executor::DockerCli,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ExperimentId {
    #[value(name = "bm-r0")]
    BmR0,
    #[value(name = "bm-r1")]
    BmR1,
    #[value(name = "bm-r2")]
    BmR2,
    #[value(name = "bm-rf0")]
    BmRf0,
    #[value(name = "bm-rf1")]
    BmRf1,
    #[value(name = "bm-d0")]
    BmD0,
    #[value(name = "bm-d1")]
    BmD1,
    #[value(name = "bm-d2")]
    BmD2,
    #[value(name = "bm-d3")]
    BmD3,
    #[value(name = "bm-df0")]
    BmDf0,
    #[value(name = "bm-df1")]
    BmDf1,
    #[value(name = "bm-dm1")]
    BmDm1,
}

impl ExperimentId {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BmR0 => "bm-r0",
            Self::BmR1 => "bm-r1",
            Self::BmR2 => "bm-r2",
            Self::BmRf0 => "bm-rf0",
            Self::BmRf1 => "bm-rf1",
            Self::BmD0 => "bm-d0",
            Self::BmD1 => "bm-d1",
            Self::BmD2 => "bm-d2",
            Self::BmD3 => "bm-d3",
            Self::BmDf0 => "bm-df0",
            Self::BmDf1 => "bm-df1",
            Self::BmDm1 => "bm-dm1",
        }
    }
}

#[derive(Debug, Serialize)]
struct BenchReport {
    experiment: String,
    orchestrator: String,
    matrix_slug: String,
    campaign_topology: String,
    backend: String,
    executor: String,
    scenario_id: String,
    primary_metric: String,
    image_ref: String,
    image_digest: Option<String>,
    agent_count: u32,
    success_rate: f64,
    starts_per_sec: Option<f64>,
    authoritative: bool,
    stats: MetricStats,
    step_timings: Vec<StepTimingReport>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct StepTimingReport {
    step_index: usize,
    op: String,
    samples_ms: Vec<f64>,
    stats: MetricStats,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Experiments => {
            println!("bm-d0   schedule enqueue→claim");
            println!("bm-d1   warm deploy → Ready (shared nginx image)");
            println!("bm-d2   teardown → Gone");
            println!("bm-d3   cold pull + deploy → Ready");
            println!("bm-df0  firehose N=50 c=4");
            println!("bm-df1  firehose N/c via --firehose-n/--firehose-c");
            println!("bm-dm1  multi-agent efficiency label (--agent-count)");
            println!("bm-r0   heartbeat round trip (in-process stub)");
            println!("bm-r1   single deploy warm stub");
            println!("bm-r2   single teardown stub");
            println!("bm-rf0  firehose N=10 stub");
            println!("bm-rf1  firehose N=50 stub");
            println!("Authoritative: AWS hardware only (see pion-bench/PERFORMANCE.md)");
        }
        Command::Run {
            experiment,
            report,
            ops,
            hardware,
            backend,
            topology,
            executor,
            firehose_n,
            firehose_c,
            agent_count,
        } => {
            let default_path = run_experiment(
                experiment,
                ops,
                hardware,
                backend,
                topology,
                executor,
                firehose_n,
                firehose_c,
                agent_count,
            )
            .await?;
            if let Some(dest) = report {
                if dest != default_path {
                    fs::copy(&default_path, &dest)?;
                    println!("copied report to {}", dest.display());
                }
            }
            println!("wrote {}", default_path.display());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_experiment(
    experiment: ExperimentId,
    ops: u32,
    hardware: HardwareArg,
    backend: BackendArg,
    topology: TopologyArg,
    executor: ExecutorArg,
    firehose_n: Option<u32>,
    firehose_c: Option<u32>,
    agent_count: u32,
) -> Result<PathBuf> {
    let hw = hardware.to_hardware();
    let matrix = MatrixSpec {
        topology: topology.to_topology(),
        executor: executor.to_executor(),
        backend: backend.to_backend(),
        telemetry: pion_testkit::TelemetryAdapter::Off,
        hardware: hw,
    };

    let edge = matches!(
        experiment,
        ExperimentId::BmD0
            | ExperimentId::BmD1
            | ExperimentId::BmD2
            | ExperimentId::BmD3
            | ExperimentId::BmDf0
            | ExperimentId::BmDf1
            | ExperimentId::BmDm1
    );
    if edge && matrix.executor == Executor::Stub && !matches!(experiment, ExperimentId::BmD0) {
        bail!(
            "{} requires --executor docker-cli (shared deployable path)",
            experiment.as_str()
        );
    }

    let iterations = match experiment {
        ExperimentId::BmRf0
        | ExperimentId::BmRf1
        | ExperimentId::BmDf0
        | ExperimentId::BmDf1
        | ExperimentId::BmDm1 => 1,
        ExperimentId::BmD1 | ExperimentId::BmD2 => ops.clamp(1, 30),
        ExperimentId::BmD3 => ops.clamp(1, 20),
        ExperimentId::BmD0 => ops.clamp(1, 50),
        _ => ops.max(1),
    };

    let mut primary_samples = Vec::new();
    let mut last_step_reports: Vec<StepTimingReport> = Vec::new();
    let mut successes = 0u32;
    let mut attempts = 0u32;
    let mut wall_for_rate: Option<f64> = None;
    let mut image_ref = bench_image_ref();
    let mut image_digest: Option<String> = None;

    let (primary_metric, primary_op): (&str, &str) = match experiment {
        ExperimentId::BmR0 => ("heartbeat_rtt_ms", "heartbeat"),
        ExperimentId::BmR1 => ("deploy_warm_ms", "claim_execute"),
        ExperimentId::BmR2 => ("teardown_ms", "claim_execute"),
        ExperimentId::BmRf0
        | ExperimentId::BmRf1
        | ExperimentId::BmDf0
        | ExperimentId::BmDf1
        | ExperimentId::BmDm1 => ("firehose_deploy_ms", "firehose_deploy"),
        ExperimentId::BmD0 => ("schedule_ms", "schedule_claim"),
        ExperimentId::BmD1 => ("time_to_ready_ms", "claim_execute"),
        ExperimentId::BmD2 => ("time_to_gone_ms", "claim_execute"),
        ExperimentId::BmD3 => ("cold_ttr_ms", "claim_execute"),
    };

    let mut scenario_id = String::new();

    for _ in 0..iterations {
        let mut session = BootstrapSession::new(matrix.clone())?;
        session.install().await?;
        let runner = ScenarioRunner::new(&session);
        let spec = match experiment {
            ExperimentId::BmR0 => ScenarioSpec::heartbeat_rtt_smoke(),
            ExperimentId::BmR1 => ScenarioSpec::enroll_deploy_smoke(),
            ExperimentId::BmR2 => ScenarioSpec::enroll_teardown_smoke(),
            ExperimentId::BmRf0 | ExperimentId::BmRf1 => ScenarioSpec::firehose_deploy_stub(),
            ExperimentId::BmD0 => ScenarioSpec::schedule_claim_smoke(),
            ExperimentId::BmD1 => ScenarioSpec::enroll_deploy_warm_docker(),
            ExperimentId::BmD2 => ScenarioSpec::enroll_teardown_gone_docker(),
            ExperimentId::BmD3 => ScenarioSpec::enroll_deploy_cold_docker(),
            ExperimentId::BmDf0 => ScenarioSpec::firehose_deploy_df0(),
            ExperimentId::BmDf1 | ExperimentId::BmDm1 => {
                let n = firehose_n.unwrap_or(50);
                let c = firehose_c.unwrap_or(4);
                ScenarioSpec::firehose_deploy_n(n, c)
            }
        };
        scenario_id = spec.id.clone();
        attempts += 1;
        let t_wall = Instant::now();
        let result = runner.run(&spec, RunMode::Benchmark).await?;
        if let Some(err) = result.error {
            bail!("{} failed: {err}", experiment.as_str());
        }
        successes += 1;
        if let Some(ref img) = result.image_ref {
            image_ref.clone_from(img);
        }
        if image_digest.is_none() {
            image_digest = resolve_image_digest(&image_ref).ok().flatten();
        }

        let samples_for_iter: Vec<f64> =
            if experiment == ExperimentId::BmR2 || experiment == ExperimentId::BmD2 {
                result
                    .step_timings
                    .iter()
                    .rev()
                    .find(|t| t.op == primary_op)
                    .map(|t| t.samples_ms.clone())
                    .unwrap_or_default()
            } else {
                result
                    .step_timings
                    .iter()
                    .find(|t| t.op == primary_op)
                    .map(|t| t.samples_ms.clone())
                    .unwrap_or_default()
            };

        if matches!(
            experiment,
            ExperimentId::BmDf0 | ExperimentId::BmDf1 | ExperimentId::BmDm1
        ) {
            let n = samples_for_iter.len() as f64;
            let secs = t_wall.elapsed().as_secs_f64().max(1e-9);
            wall_for_rate = Some(n / secs);
        }

        primary_samples.extend(samples_for_iter);

        last_step_reports = result
            .step_timings
            .into_iter()
            .map(|t| {
                let st = MetricStats::summarize(t.samples_ms.clone());
                StepTimingReport {
                    step_index: t.step_index,
                    op: t.op,
                    samples_ms: t.samples_ms,
                    stats: st,
                }
            })
            .collect();
    }

    let success_rate = if attempts == 0 {
        0.0
    } else {
        f64::from(successes) / f64::from(attempts)
    };
    if matches!(
        experiment,
        ExperimentId::BmDf0 | ExperimentId::BmDf1 | ExperimentId::BmDm1
    ) && success_rate < 0.99
    {
        bail!(
            "{} success_rate {success_rate:.3} below 0.99 SLO",
            experiment.as_str()
        );
    }

    let stats = MetricStats::summarize(primary_samples);
    let orch_slug = format!("{}-pion-{}", experiment.as_str(), matrix.report_slug());
    let report = BenchReport {
        experiment: experiment.as_str().into(),
        orchestrator: "pion".into(),
        matrix_slug: matrix.report_slug(),
        campaign_topology: matrix.topology.as_str().into(),
        backend: matrix.backend.as_str().into(),
        executor: matrix.executor.as_str().into(),
        scenario_id,
        primary_metric: primary_metric.into(),
        image_ref,
        image_digest,
        agent_count,
        success_rate,
        starts_per_sec: wall_for_rate,
        authoritative: hw.is_authoritative(),
        stats,
        step_timings: last_step_reports,
        error: None,
    };
    write_report(&orch_slug, &report)
}

fn write_report(slug: &str, report: &BenchReport) -> Result<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("reports");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{slug}.json"));
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&path, json)?;
    println!(
        "{} {} p50={:.3}ms p95={:.3}ms n={} auth={} starts/s={:?}",
        report.experiment,
        report.primary_metric,
        report.stats.p50,
        report.stats.p95,
        report.stats.count,
        report.authoritative,
        report.starts_per_sec
    );
    Ok(path)
}
