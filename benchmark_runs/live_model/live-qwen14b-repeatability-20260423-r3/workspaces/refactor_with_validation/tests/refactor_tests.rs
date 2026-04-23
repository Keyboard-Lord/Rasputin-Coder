use bench_refactor::{label_team, label_user};

#[test]
fn labels_are_stable() {
    assert_eq!(label_user("ada"), "user:ada");
    assert_eq!(label_team("core"), "team:core");
}
