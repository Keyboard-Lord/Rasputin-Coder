pub fn existing() -> bool {
    true
}

#[derive(Debug, PartialEq)]
pub struct Setting {
    pub key: String,
    pub value: String,
}

pub fn parse_setting(input: &str) -> Option<Setting> {
    let parts: Vec<&str> = input.split('=').collect();
    if parts.len() == 2 {
        let key = parts[0].trim();
        let value = parts[1].trim();
        if !key.is_empty() {
            return Some(Setting {
                key: key.to_string(),
                value: value.to_string(),
            });
        }
    }
    None
}
