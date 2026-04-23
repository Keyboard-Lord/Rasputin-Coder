pub mod token {
    pub fn parse_bearer_token(header: &str) -> Option<String> {
        if header.starts_with("Bearer ") {
            Some(header[7..].to_string())
        } else {
            None
        }
    }
}