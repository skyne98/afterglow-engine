pub fn hello() -> &'static str {
    "hello from afterglow-engine"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_works() {
        assert_eq!(hello(), "hello from afterglow-engine");
    }
}
