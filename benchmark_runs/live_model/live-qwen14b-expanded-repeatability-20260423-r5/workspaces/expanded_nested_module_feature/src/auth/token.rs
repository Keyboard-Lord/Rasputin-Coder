pub fn is_bearer(header: &str) -> bool {
    bearer_value(header).is_some()
}

pub fn bearer_value(header: &str) -> Option<String> {
    let value = header.strip_prefix("Bearer")?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
