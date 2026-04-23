pub fn answer() -> i32 {
    42
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_is_42() {
        assert_eq!(answer(), 42);
    }
}