use bench_settings::settings::loader::load_defaults;

#[test]
fn retry_default_is_updated() {
    let settings = load_defaults();
    assert_eq!(settings.timeout_ms, 2500);
    assert_eq!(settings.max_retries, 5);
}
