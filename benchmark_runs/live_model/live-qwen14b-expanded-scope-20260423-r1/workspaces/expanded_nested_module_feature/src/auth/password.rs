pub fn mask_password(input: &str) -> String {
    "*".repeat(input.len().min(8))
}
