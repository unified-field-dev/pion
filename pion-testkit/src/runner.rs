//! Shared scenario executor for e2e (correctness) and bench (timings).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use parton::{
    execute_claimed_node_action, ClaimedNodeActionBody, ContainerStatusReport,
    ContainerStatusSummary, DockerCliActionExecutor, HostCapabilities, NodeHeartbeatReport,
};
use pion::{
    claim_pending_node_action, create_host_enrollment, enqueue_node_action, ingest_agent_heartbeat,
    report_node_action_result, upsert_node_action_capability, ClaimedNodeAction,
    ContainerActionKind, DeployHandoffActionRequest, ReportNodeActionResult,
    TeardownHandoffActionRequest, NODE_ACTION_KIND_DEPLOY_HANDOFF,
    NODE_ACTION_KIND_TEARDOWN_HANDOFF,
};
use valence::Valence;

use crate::bench_image::{bench_image_ref, bench_port_mapping, pre_pull_image_async};
use crate::bootstrap::BootstrapSession;
use crate::matrix::{Executor, Topology};
use crate::scenario::{ScenarioSpec, ScenarioStep};

/// Driver mode: assert on outcomes vs collect timings only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Fail the run when assertions are not met.
    Correctness,
    /// Collect timings without failing soft delivery counts (still fails hard errors).
    Benchmark,
}

/// Per-step timing samples (milliseconds).
#[derive(Debug, Clone)]
pub struct StepTiming {
    /// Index of the scenario step.
    pub step_index: usize,
    /// Operation label (for example `heartbeat`, `claim_execute`).
    pub op: String,
    /// Per-op samples in milliseconds.
    pub samples_ms: Vec<f64>,
}

/// Outcome of running one [`ScenarioSpec`].
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// Scenario identifier.
    pub scenario_id: String,
    /// Matrix row slug for reports.
    pub matrix_slug: String,
    /// Per-step timing samples.
    pub step_timings: Vec<StepTiming>,
    /// Failure message when the run did not meet assertions.
    pub error: Option<String>,
    /// Image ref used for deploy rows (when set).
    pub image_ref: Option<String>,
}

struct HarnessState {
    node_id: String,
    cell_id: String,
    enrollment_token: Option<String>,
    enrolled: bool,
    healthy: bool,
    /// Stub / tracked container refs currently "running".
    running: HashSet<String>,
    container_ref: String,
    image_ref: String,
    host_port: u16,
    last_command_id: Option<String>,
    last_attempt: i64,
    executor: Executor,
}

impl HarnessState {
    fn new(executor: Executor) -> Self {
        let run_id = format!(
            "{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis())
        );
        // Unique per run so persistent Hybrid Postgres does not block on stale correlation sequences.
        let suffix: String = run_id.chars().rev().take(10).collect();
        let port_offset: u16 = suffix.bytes().fold(0u16, |acc, b| {
            acc.wrapping_mul(31).wrapping_add(u16::from(b))
        }) % 2000;
        Self {
            node_id: format!("pion-lab-node-{suffix}"),
            cell_id: "local-default".into(),
            enrollment_token: None,
            enrolled: false,
            healthy: false,
            running: HashSet::new(),
            container_ref: format!("lab-handoff-{suffix}"),
            image_ref: bench_image_ref(),
            host_port: 18_000 + port_offset,
            last_command_id: None,
            last_attempt: 0,
            executor,
        }
    }

    fn deploy_request(
        &self,
        container_ref: String,
        transfer_id: Option<String>,
    ) -> DeployHandoffActionRequest {
        DeployHandoffActionRequest {
            node_id: self.node_id.clone(),
            container_ref,
            image_ref: self.image_ref.clone(),
            env_vars: vec![],
            secret_env_vars: vec![],
            port_mappings: vec![bench_port_mapping(self.host_port)],
            extra_hosts: vec![],
            volume_mounts: vec![],
            network: None,
            health_url: Some(format!("http://127.0.0.1:{}/", self.host_port)),
            import_base_url: None,
            transfer_id,
        }
    }
}

/// Executes declarative scenarios against a bootstrapped session.
pub struct ScenarioRunner<'a> {
    session: &'a BootstrapSession,
}

impl<'a> ScenarioRunner<'a> {
    /// Bind a runner to an installed bootstrap session.
    #[must_use]
    pub const fn new(session: &'a BootstrapSession) -> Self {
        Self { session }
    }

    /// Run all steps in `spec`, returning timings and optional error.
    ///
    /// `IsolatedHarness`, `LocalDocker`, `AwsSameRegion` and `AwsWan` topologies with the Stub or `DockerCli` executor are supported.
    /// `DockerCli` claims call Parton [`DockerCliActionExecutor`] on the blocking pool and wait
    /// for HTTP Ready on the published port.
    ///
    /// `AwsWan` executes identically to `AwsSameRegion` (Docker CLI is invoked against whatever
    /// host the `pion-bench`/`pion-e2e` process is running on); the distinction is the physical
    /// host the *caller* runs this process on (cross-region agent host vs. CP host) and the
    /// topology label attached to reports.
    ///
    /// # Errors
    ///
    /// Returns an error if bootstrap is not ready or a hard step failure occurs.
    pub async fn run(&self, spec: &ScenarioSpec, mode: RunMode) -> Result<ScenarioResult> {
        if !self.session.is_ready() {
            bail!("BootstrapSession::install must succeed before running scenarios");
        }

        let matrix = self.session.matrix();
        let matrix_slug = matrix.report_slug();

        if matrix.topology != Topology::IsolatedHarness
            && matrix.topology != Topology::AwsSameRegion
            && matrix.topology != Topology::LocalDocker
            && matrix.topology != Topology::AwsWan
        {
            return Ok(ScenarioResult {
                scenario_id: spec.id.clone(),
                matrix_slug,
                step_timings: vec![],
                error: Some(format!(
                    "topology {:?} not implemented in runner",
                    matrix.topology
                )),
                image_ref: None,
            });
        }
        if matrix.executor != Executor::Stub && matrix.executor != Executor::DockerCli {
            return Ok(ScenarioResult {
                scenario_id: spec.id.clone(),
                matrix_slug,
                step_timings: vec![],
                error: Some(format!(
                    "executor {:?} not implemented in runner",
                    matrix.executor
                )),
                image_ref: None,
            });
        }

        let valence = self.session.valence("pion_testkit_scenario")?;
        let mut lab = HarnessState::new(matrix.executor);
        let mut step_timings = Vec::new();

        for (step_index, step) in spec.steps.iter().enumerate() {
            match self
                .run_step(&valence, &mut lab, step_index, step, &mut step_timings)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    let msg = e.to_string();
                    // Docker-cli edge rows must not soft-skip failed deploys (would yield empty stats).
                    if mode == RunMode::Correctness
                        || matrix.executor == Executor::DockerCli
                        || is_hard_error(&msg)
                    {
                        let image_ref = Some(lab.image_ref.clone());
                        docker_rm_force(&lab.container_ref);
                        return Ok(ScenarioResult {
                            scenario_id: spec.id.clone(),
                            matrix_slug,
                            step_timings,
                            error: Some(msg),
                            image_ref,
                        });
                    }
                }
            }
        }

        Self::run_finish_cleanup(&lab);
        Ok(ScenarioResult {
            scenario_id: spec.id.clone(),
            matrix_slug,
            step_timings,
            error: None,
            image_ref: Some(lab.image_ref.clone()),
        })
    }

    fn run_finish_cleanup(lab: &HarnessState) {
        if lab.executor == Executor::DockerCli {
            docker_rm_force(&lab.container_ref);
            for cref in &lab.running {
                docker_rm_force(cref);
            }
        }
    }

    async fn run_step(
        &self,
        valence: &Valence,
        lab: &mut HarnessState,
        step_index: usize,
        step: &ScenarioStep,
        timings: &mut Vec<StepTiming>,
    ) -> Result<()> {
        match step {
            ScenarioStep::EnrollAgent => {
                let t0 = Instant::now();
                let (_, token) = create_host_enrollment(
                    valence,
                    "127.0.0.1".to_string(),
                    lab.cell_id.clone(),
                    "default".to_string(),
                    86_400,
                )
                .await?;
                lab.enrollment_token = Some(token);
                lab.enrolled = true;
                timings.push(StepTiming {
                    step_index,
                    op: "enroll".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::HeartbeatUntilHealthy => {
                if !lab.enrolled {
                    bail!("HeartbeatUntilHealthy requires EnrollAgent first");
                }
                let t0 = Instant::now();
                heartbeat_once(valence, lab).await?;
                // Node action capabilities default to disabled; the harness plays the
                // role of the operator opting this node into the actions scenarios exercise.
                enable_lab_node_capabilities(valence, &lab.node_id).await?;
                lab.healthy = true;
                lab.enrollment_token = None;
                timings.push(StepTiming {
                    step_index,
                    op: "heartbeat".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::PrePullImage => {
                let t0 = Instant::now();
                let image = lab.image_ref.clone();
                pre_pull_image_async(image).await.context("PrePullImage")?;
                timings.push(StepTiming {
                    step_index,
                    op: "pre_pull".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::EnqueueDeploy => {
                ensure_healthy(lab)?;
                let t0 = Instant::now();
                let req = lab.deploy_request(lab.container_ref.clone(), Some("lab-xfer".into()));
                let payload = serde_json::to_value(&req)?;
                let cmd_id = enqueue_node_action(
                    &lab.node_id,
                    &lab.cell_id,
                    NODE_ACTION_KIND_DEPLOY_HANDOFF,
                    payload,
                    3,
                    Some("lab:deploy"),
                    Some(1),
                    valence,
                )
                .await?;
                lab.last_command_id = Some(cmd_id);
                timings.push(StepTiming {
                    step_index,
                    op: "enqueue_deploy".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::ClaimAndAck => {
                ensure_healthy(lab)?;
                let t0 = Instant::now();
                let claimed = claim_pending_node_action(&lab.node_id, 120, valence)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no pending action to claim"))?;
                lab.last_command_id = Some(claimed.command_id.clone());
                lab.last_attempt = claimed.attempt;
                report_node_action_result(
                    ReportNodeActionResult {
                        command_id: claimed.command_id,
                        node_id: lab.node_id.clone(),
                        attempt: claimed.attempt,
                        success: true,
                        stdout: Some("ack only".into()),
                        stderr: None,
                        error_summary: None,
                        payload_json: Some(serde_json::json!({ "executor": "ack" })),
                    },
                    valence,
                )
                .await?;
                timings.push(StepTiming {
                    step_index,
                    op: "schedule_claim".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::ClaimAndExecute => {
                ensure_healthy(lab)?;
                let t0 = Instant::now();
                let claimed = claim_pending_node_action(&lab.node_id, 120, valence)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("no pending action to claim"))?;
                lab.last_command_id = Some(claimed.command_id.clone());
                lab.last_attempt = claimed.attempt;

                match lab.executor {
                    Executor::Stub => {
                        stub_mutate_running(lab, &claimed)?;
                        report_node_action_result(stub_report(&claimed, &lab.node_id), valence)
                            .await?;
                    }
                    Executor::DockerCli => {
                        let report = docker_claim_execute(&claimed, &lab.node_id).await?;
                        let kind = claimed.action_kind.as_str();
                        let cref = container_ref_from_payload(&claimed.payload_json)
                            .unwrap_or_else(|| lab.container_ref.clone());
                        if report.success {
                            if kind == NODE_ACTION_KIND_DEPLOY_HANDOFF {
                                wait_http_ready(lab.host_port, Duration::from_secs(30)).await?;
                                lab.running.insert(cref);
                            } else if kind == NODE_ACTION_KIND_TEARDOWN_HANDOFF {
                                // Stop leaves the container; remove so Gone matches the contract.
                                docker_rm_force_async(cref.clone()).await?;
                                lab.running.remove(&cref);
                            }
                        }
                        let failed = !report.success;
                        let err_summary = report
                            .error_summary
                            .clone()
                            .unwrap_or_else(|| "unknown".into());
                        report_node_action_result(
                            ReportNodeActionResult {
                                command_id: report.command_id,
                                node_id: report.node_id,
                                attempt: report.attempt,
                                success: report.success,
                                stdout: report.stdout,
                                stderr: report.stderr,
                                error_summary: report.error_summary,
                                payload_json: report.payload_json,
                            },
                            valence,
                        )
                        .await?;
                        if failed {
                            bail!("docker-cli action failed: {err_summary}");
                        }
                    }
                }

                timings.push(StepTiming {
                    step_index,
                    op: "claim_execute".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::AssertContainerRunning => {
                match lab.executor {
                    Executor::Stub => {
                        if !lab.running.contains(&lab.container_ref) {
                            bail!(
                                "expected container `{}` running; stub set={:?}",
                                lab.container_ref,
                                lab.running
                            );
                        }
                    }
                    Executor::DockerCli => {
                        if !docker_is_running(&lab.container_ref)? {
                            bail!("expected container `{}` running", lab.container_ref);
                        }
                        wait_http_ready(lab.host_port, Duration::from_secs(5)).await?;
                    }
                }
                timings.push(StepTiming {
                    step_index,
                    op: "assert_running".into(),
                    samples_ms: vec![0.0],
                });
            }
            ScenarioStep::EnqueueTeardown => {
                ensure_healthy(lab)?;
                let t0 = Instant::now();
                let req = TeardownHandoffActionRequest {
                    node_id: lab.node_id.clone(),
                    container_ref: lab.container_ref.clone(),
                    remove_volumes: false,
                    expected_container_id: None,
                };
                let payload = serde_json::to_value(&req)?;
                let cmd_id = enqueue_node_action(
                    &lab.node_id,
                    &lab.cell_id,
                    NODE_ACTION_KIND_TEARDOWN_HANDOFF,
                    payload,
                    3,
                    Some("lab:teardown"),
                    Some(1),
                    valence,
                )
                .await?;
                lab.last_command_id = Some(cmd_id);
                timings.push(StepTiming {
                    step_index,
                    op: "enqueue_teardown".into(),
                    samples_ms: vec![elapsed_ms(t0)],
                });
            }
            ScenarioStep::AssertContainerGone => {
                match lab.executor {
                    Executor::Stub => {
                        if lab.running.contains(&lab.container_ref) {
                            bail!(
                                "expected container `{}` gone; still in stub set",
                                lab.container_ref
                            );
                        }
                    }
                    Executor::DockerCli => {
                        if docker_container_exists(&lab.container_ref)? {
                            bail!(
                                "expected container `{}` gone; still present",
                                lab.container_ref
                            );
                        }
                    }
                }
                timings.push(StepTiming {
                    step_index,
                    op: "assert_gone".into(),
                    samples_ms: vec![0.0],
                });
            }
            ScenarioStep::FirehoseDeploy { n, concurrency } => {
                ensure_healthy(lab)?;
                let t0 = Instant::now();
                let conc = usize::try_from((*concurrency).max(1)).unwrap_or(1);
                let mut samples = Vec::with_capacity(*n as usize);
                for batch_start in (0..*n).step_by(conc) {
                    let batch_end = batch_start
                        .saturating_add(u32::try_from(conc).unwrap_or(u32::MAX))
                        .min(*n);
                    for i in batch_start..batch_end {
                        let cref = format!("{}-{i}", lab.container_ref);
                        let host_port =
                            19_000u16.saturating_add(u16::try_from(i).unwrap_or(u16::MAX));
                        let mut req =
                            lab.deploy_request(cref.clone(), Some(format!("lab-xfer-{i}")));
                        req.port_mappings = vec![bench_port_mapping(host_port)];
                        req.health_url = Some(format!("http://127.0.0.1:{host_port}/"));
                        let payload = serde_json::to_value(&req)?;
                        let sample_t0 = Instant::now();
                        enqueue_node_action(
                            &lab.node_id,
                            &lab.cell_id,
                            NODE_ACTION_KIND_DEPLOY_HANDOFF,
                            payload,
                            3,
                            Some(&format!("lab:firehose:{i}")),
                            Some(i64::from(i) + 1),
                            valence,
                        )
                        .await?;
                        let claimed = claim_pending_node_action(&lab.node_id, 120, valence)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("firehose: no claim"))?;
                        match lab.executor {
                            Executor::Stub => {
                                lab.running.insert(cref);
                                report_node_action_result(
                                    stub_report(&claimed, &lab.node_id),
                                    valence,
                                )
                                .await?;
                            }
                            Executor::DockerCli => {
                                // Temporarily set host port for ready wait.
                                let prev_port = lab.host_port;
                                lab.host_port = host_port;
                                let report = docker_claim_execute(&claimed, &lab.node_id).await?;
                                if report.success {
                                    wait_http_ready(host_port, Duration::from_secs(30)).await?;
                                    lab.running.insert(cref);
                                }
                                report_node_action_result(
                                    ReportNodeActionResult {
                                        command_id: report.command_id,
                                        node_id: report.node_id,
                                        attempt: report.attempt,
                                        success: report.success,
                                        stdout: report.stdout,
                                        stderr: report.stderr,
                                        error_summary: report.error_summary.clone(),
                                        payload_json: report.payload_json,
                                    },
                                    valence,
                                )
                                .await?;
                                lab.host_port = prev_port;
                                if !report.success {
                                    bail!(
                                        "firehose deploy failed: {}",
                                        report.error_summary.unwrap_or_else(|| "unknown".into())
                                    );
                                }
                            }
                        }
                        samples.push(elapsed_ms(sample_t0));
                    }
                }
                let _ = elapsed_ms(t0);
                timings.push(StepTiming {
                    step_index,
                    op: "firehose_deploy".into(),
                    samples_ms: samples,
                });
            }
            ScenarioStep::FirehoseTeardown { n, concurrency } => {
                ensure_healthy(lab)?;
                let conc = usize::try_from((*concurrency).max(1)).unwrap_or(1);
                let mut samples = Vec::with_capacity(*n as usize);
                for batch_start in (0..*n).step_by(conc) {
                    let batch_end = batch_start
                        .saturating_add(u32::try_from(conc).unwrap_or(u32::MAX))
                        .min(*n);
                    for i in batch_start..batch_end {
                        let cref = format!("{}-{i}", lab.container_ref);
                        let req = TeardownHandoffActionRequest {
                            node_id: lab.node_id.clone(),
                            container_ref: cref.clone(),
                            remove_volumes: false,
                            expected_container_id: None,
                        };
                        let payload = serde_json::to_value(&req)?;
                        let sample_t0 = Instant::now();
                        enqueue_node_action(
                            &lab.node_id,
                            &lab.cell_id,
                            NODE_ACTION_KIND_TEARDOWN_HANDOFF,
                            payload,
                            3,
                            Some(&format!("lab:firehose-td:{i}")),
                            Some(i64::from(i) + 1),
                            valence,
                        )
                        .await?;
                        let claimed = claim_pending_node_action(&lab.node_id, 120, valence)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("firehose teardown: no claim"))?;
                        match lab.executor {
                            Executor::Stub => {
                                lab.running.remove(&cref);
                                report_node_action_result(
                                    stub_report(&claimed, &lab.node_id),
                                    valence,
                                )
                                .await?;
                            }
                            Executor::DockerCli => {
                                let report = docker_claim_execute(&claimed, &lab.node_id).await?;
                                if report.success {
                                    docker_rm_force_async(cref.clone()).await?;
                                    lab.running.remove(&cref);
                                }
                                report_node_action_result(
                                    ReportNodeActionResult {
                                        command_id: report.command_id,
                                        node_id: report.node_id,
                                        attempt: report.attempt,
                                        success: report.success,
                                        stdout: report.stdout,
                                        stderr: report.stderr,
                                        error_summary: report.error_summary.clone(),
                                        payload_json: report.payload_json,
                                    },
                                    valence,
                                )
                                .await?;
                                if !report.success {
                                    bail!(
                                        "firehose teardown failed: {}",
                                        report.error_summary.unwrap_or_else(|| "unknown".into())
                                    );
                                }
                            }
                        }
                        samples.push(elapsed_ms(sample_t0));
                    }
                }
                timings.push(StepTiming {
                    step_index,
                    op: "firehose_teardown".into(),
                    samples_ms: samples,
                });
            }
            ScenarioStep::ExtendLease
            | ScenarioStep::CancelAction
            | ScenarioStep::ApplyReEnroll
            | ScenarioStep::ApplyRevoke => {
                timings.push(StepTiming {
                    step_index,
                    op: format!("{step:?}").to_ascii_lowercase(),
                    samples_ms: vec![0.0],
                });
            }
        }
        Ok(())
    }
}

fn ensure_healthy(lab: &HarnessState) -> Result<()> {
    if !lab.healthy {
        bail!("node not healthy; run HeartbeatUntilHealthy first");
    }
    Ok(())
}

fn stub_mutate_running(lab: &mut HarnessState, claimed: &ClaimedNodeAction) -> Result<()> {
    match claimed.action_kind.as_str() {
        NODE_ACTION_KIND_DEPLOY_HANDOFF => {
            let cref = container_ref_from_payload(&claimed.payload_json)
                .unwrap_or_else(|| lab.container_ref.clone());
            lab.running.insert(cref);
        }
        NODE_ACTION_KIND_TEARDOWN_HANDOFF => {
            let cref = container_ref_from_payload(&claimed.payload_json)
                .unwrap_or_else(|| lab.container_ref.clone());
            lab.running.remove(&cref);
        }
        other => bail!("stub executor does not handle action_kind {other}"),
    }
    Ok(())
}

fn stub_report(claimed: &ClaimedNodeAction, node_id: &str) -> ReportNodeActionResult {
    ReportNodeActionResult {
        command_id: claimed.command_id.clone(),
        node_id: node_id.to_string(),
        attempt: claimed.attempt,
        success: true,
        stdout: Some("stub ok".into()),
        stderr: None,
        error_summary: None,
        payload_json: Some(serde_json::json!({ "executor": "stub" })),
    }
}

fn container_ref_from_payload(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("container_ref")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

async fn docker_claim_execute(
    claimed: &ClaimedNodeAction,
    node_id: &str,
) -> Result<parton::ReportRequestBody> {
    let body = ClaimedNodeActionBody {
        command_id: claimed.command_id.clone(),
        action_kind: claimed.action_kind.clone(),
        payload_json: claimed.payload_json.clone(),
        attempt: claimed.attempt,
        correlation_key: claimed.correlation_key.clone(),
        sequence: claimed.sequence,
    };
    // Docker CLI runs on spawn_blocking inside parton; Arc satisfies Send+Sync+'static.
    execute_claimed_node_action(Arc::new(DockerCliActionExecutor), &body, node_id).await
}

async fn wait_http_ready(host_port: u16, timeout: Duration) -> Result<()> {
    let addr = format!("127.0.0.1:{host_port}");
    let socket: std::net::SocketAddr = addr
        .parse()
        .with_context(|| format!("invalid ready addr {addr}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        let tcp_ok = {
            let sock = socket;
            tokio::task::spawn_blocking(move || {
                std::net::TcpStream::connect_timeout(&sock, Duration::from_millis(200)).is_ok()
            })
            .await
            .unwrap_or(false)
        };
        if tcp_ok {
            let url = format!("http://{addr}/");
            let http_ok = tokio::task::spawn_blocking(move || {
                std::process::Command::new("curl")
                    .args(["-fsS", "--max-time", "2", &url])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map_or(true, |s| s.success())
            })
            .await
            .unwrap_or(true);
            if http_ok {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            bail!("Ready probe timed out for http://{addr}/");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn docker_is_running(container_ref: &str) -> Result<bool> {
    let output = std::process::Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_ref])
        .output()
        .context("docker inspect")?;
    if !output.status.success() {
        return Ok(false);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

fn docker_container_exists(container_ref: &str) -> Result<bool> {
    let output = std::process::Command::new("docker")
        .args(["inspect", container_ref])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("docker inspect exists")?;
    Ok(output.success())
}

fn docker_rm_force(container_ref: &str) {
    let _ = std::process::Command::new("docker")
        .args(["rm", "-f", container_ref])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

async fn docker_rm_force_async(container_ref: String) -> Result<()> {
    tokio::task::spawn_blocking(move || docker_rm_force(&container_ref))
        .await
        .context("docker rm join")?;
    Ok(())
}

/// Opts a freshly-heartbeating harness node into the deploy/stop capabilities the scenario steps
/// exercise (`deploy_handoff` maps to `Deploy`, `teardown_handoff` maps to `Stop`).
async fn enable_lab_node_capabilities(valence: &Valence, node_id: &str) -> Result<()> {
    for action in [ContainerActionKind::Deploy, ContainerActionKind::Stop] {
        upsert_node_action_capability(
            node_id,
            action,
            true,
            "pion-testkit",
            Some("lab/bench scenario opt-in"),
            None,
            valence,
        )
        .await?;
    }
    Ok(())
}

async fn heartbeat_once(valence: &Valence, lab: &HarnessState) -> Result<()> {
    let report = NodeHeartbeatReport {
        node_id: lab.node_id.clone(),
        cell_id: lab.cell_id.clone(),
        capabilities: HostCapabilities {
            hostname: format!("{}.local", lab.node_id),
            arch: "x86_64".to_string(),
            cpu_logical: 4,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            mounts: vec![],
            labels: serde_json::json!({}),
        },
        containers: ContainerStatusReport {
            containers: vec![],
            summary: ContainerStatusSummary {
                running: lab.running.len() as u64,
                exited: 0,
                unhealthy: 0,
            },
        },
        observed_at: Utc::now(),
        enrollment_token: lab.enrollment_token.clone(),
        applied_directive_token: None,
        apply_failed: None,
    };
    ingest_agent_heartbeat(&report, Some("127.0.0.1"), valence).await?;
    Ok(())
}

fn elapsed_ms(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0
}

fn is_hard_error(msg: &str) -> bool {
    msg.contains("not implemented") || msg.contains("requires") || msg.contains("docker-cli")
}
