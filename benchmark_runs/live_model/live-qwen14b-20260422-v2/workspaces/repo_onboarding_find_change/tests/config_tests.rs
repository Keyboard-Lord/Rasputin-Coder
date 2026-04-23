use bench_find_change::config::default_timeout_ms;

#[test]
fn timeout_is_updated() {
    assert_eq!(default_timeout_ms(), 5000);
}
