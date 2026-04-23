pub mod validation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingError {
    MissingSeparator,
    EmptyKey,
    InvalidKey,
}

pub fn parse_and_validate_setting(input: &str) -> Result<Setting, SettingError> {
    let (key, value) = input
        .split_once('=')
        .ok_or(SettingError::MissingSeparator)?;
    let key = key.trim();
    if key.is_empty() {
        return Err(SettingError::EmptyKey);
    }
    if !validation::is_valid_key(key) {
        return Err(SettingError::InvalidKey);
    }
    Ok(Setting {
        key: key.to_string(),
        value: value.trim().to_string(),
    })
}
