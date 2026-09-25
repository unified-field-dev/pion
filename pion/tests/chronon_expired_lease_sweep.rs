//! Feature `chronon`: expired-lease sweep job name matches [`pion::chronon_hooks`].

#![cfg(feature = "chronon")]

use pion::chronon_hooks::EXPIRED_LEASE_SWEEP;

#[test]
fn expired_lease_sweep_job_name_matches_hooks_spec() {
    assert_eq!(
        EXPIRED_LEASE_SWEEP.name,
        "pion.node_actions.expired_lease_sweep"
    );
    assert_eq!(EXPIRED_LEASE_SWEEP.cron_expr, "*/2 * * * *");
}

#[test]
fn expired_lease_sweep_script_module_is_linked() {
    // Touch the module so inventory `submit!` from the script macro is linked into this binary.
    let _ = std::any::type_name_of_val(&pion::scripts::pion_node_actions_expired_lease_sweep);
}
