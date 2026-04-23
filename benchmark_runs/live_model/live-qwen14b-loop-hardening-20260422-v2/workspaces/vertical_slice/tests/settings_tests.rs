use bench_vertical::parse_setting;

#[test]
fn parses_setting() {
    let setting = parse_setting(" name = Ada ").expect("setting");
    assert_eq!(setting.key, "name");
    assert_eq!(setting.value, "Ada");
}

#[test]
fn rejects_empty_key() {
    assert!(parse_setting(" = Ada ").is_none());
}
