/// Returns a greeting string.
pub fn hello(name: &str) -> String {
    format!("Hello, {}!", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hello_returns_greeting() {
        assert_eq!(hello("World"), "Hello, World!");
    }
}
