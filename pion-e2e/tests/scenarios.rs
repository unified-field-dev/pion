//! In-process harness + stub executor: enroll → deploy claim/report smoke.

use pion_testkit::{
    BootstrapSession, Executor, Hardware, MatrixSpec, RunMode, ScenarioRunner, ScenarioSpec,
    Topology,
};

#[tokio::test]
async fn enroll_deploy_smoke_isolated_harness_stub() -> anyhow::Result<()> {
    let mut session = BootstrapSession::new(MatrixSpec::isolated_harness_stub())?;
    session.install().await?;
    let runner = ScenarioRunner::new(&session);
    let result = runner
        .run(&ScenarioSpec::enroll_deploy_smoke(), RunMode::Correctness)
        .await?;
    assert!(
        result.error.is_none(),
        "scenario failed: {:?}",
        result.error
    );
    assert!(
        result.step_timings.iter().any(|t| t.op == "claim_execute"),
        "expected claim_execute timing"
    );
    Ok(())
}

#[tokio::test]
async fn heartbeat_rtt_smoke_isolated_harness_stub() -> anyhow::Result<()> {
    let mut session = BootstrapSession::new(MatrixSpec::isolated_harness_stub())?;
    session.install().await?;
    let runner = ScenarioRunner::new(&session);
    let result = runner
        .run(&ScenarioSpec::heartbeat_rtt_smoke(), RunMode::Correctness)
        .await?;
    assert!(
        result.error.is_none(),
        "scenario failed: {:?}",
        result.error
    );
    Ok(())
}

#[tokio::test]
async fn heartbeat_rtt_smoke_aws_wan_stub() -> anyhow::Result<()> {
    // AwsWan is executed like AwsSameRegion (Docker CLI/HTTP run against whatever host this
    // process is on); this asserts the runner no longer rejects the topology outright (see
    // Remote multi-host docker-cli topology is out of scope for this in-process scenario).
    let matrix = MatrixSpec {
        topology: Topology::AwsWan,
        executor: Executor::Stub,
        ..MatrixSpec::isolated_harness_stub().with_hardware(Hardware::AwsC6iLarge)
    };
    let mut session = BootstrapSession::new(matrix)?;
    session.install().await?;
    let runner = ScenarioRunner::new(&session);
    let result = runner
        .run(&ScenarioSpec::heartbeat_rtt_smoke(), RunMode::Correctness)
        .await?;
    assert!(
        result.error.is_none(),
        "AwsWan topology should be supported by the runner: {:?}",
        result.error
    );
    Ok(())
}

#[test]
fn scenario_enroll_deploy_smoke_roundtrips_json() {
    let spec = ScenarioSpec::enroll_deploy_smoke();
    let json = serde_json::to_string(&spec).expect("serialize");
    let back: ScenarioSpec = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(spec, back);
}

#[test]
fn matrix_isolated_harness_stub_slug() {
    let slug = MatrixSpec::isolated_harness_stub().report_slug();
    assert!(slug.contains("isolated-harness"));
    assert!(slug.contains("stub"));
}
