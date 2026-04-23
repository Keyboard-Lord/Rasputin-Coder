use bench_settings_slice::{parse_and_validate_setting, SettingError};

#[test]
fn parses_valid_setting() {
    let setting = parse_and_validate_setting(" service_name = Rasputin ").expect("setting");
    assert_eq!(setting.key, "service_name");
    assert_eq!(setting.value, "Rasputin");
}

#[test]
fn rejects_empty_key() {
    assert_eq!(parse_and_validate_setting(" = value "), Err(SettingError::EmptyKey));
}

#[test]
fn rejects_invalid_key() {
    assert_eq!(parse_and_validate_setting("service-name=value"), Err(SettingError::InvalidKey));
}
