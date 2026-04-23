fn format_prefix(prefix: &str, name: &str) -> String {
    format!("{}:{}", prefix, name)
}

pub fn label_user(name: &str) -> String {
    format_prefix("user", name)
}

pub fn label_team(name: &str) -> String {
    format_prefix("team", name)
}
