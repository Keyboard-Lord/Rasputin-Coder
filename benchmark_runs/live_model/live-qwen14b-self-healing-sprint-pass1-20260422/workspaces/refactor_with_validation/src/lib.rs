fn format_with_prefix(prefix: &str, name: &str) -> String {
    format!("{}:{}", prefix, name)
}

pub fn label_user(name: &str) -> String {
    format_with_prefix("user", name)
}

pub fn label_team(name: &str) -> String {
    format_with_prefix("team", name)
}
