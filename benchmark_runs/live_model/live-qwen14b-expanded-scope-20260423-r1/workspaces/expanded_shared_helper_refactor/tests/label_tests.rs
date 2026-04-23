use bench_refactor_deep::{audit::label_audit, team::label_team, user::label_user};

#[test]
fn labels_are_stable() {
    assert_eq!(label_user("ada"), "user:ada");
    assert_eq!(label_team("core"), "team:core");
    assert_eq!(label_audit("login"), "audit:login");
}
