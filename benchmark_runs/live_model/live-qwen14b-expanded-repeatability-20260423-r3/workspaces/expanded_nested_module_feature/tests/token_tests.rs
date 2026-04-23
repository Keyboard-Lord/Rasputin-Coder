use bench_nested::auth::token::{bearer_value, is_bearer};

#[test]
fn detects_bearer_header() {
    assert!(is_bearer("Bearer abc123"));
    assert!(!is_bearer("Basic abc123"));
}

#[test]
fn extracts_bearer_value() {
    assert_eq!(bearer_value("Bearer abc123"), Some("abc123".to_string()));
    assert_eq!(bearer_value("Bearer   spaced"), Some("spaced".to_string()));
    assert_eq!(bearer_value("Bearer"), None);
}
