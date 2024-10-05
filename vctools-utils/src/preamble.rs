
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn err_from_str(msg: &str) -> Box<dyn std::error::Error> {
    msg.into()
}

/// Run `f` and prefix any errors with the string returned by `prefix`.
pub fn try_forward<'a, F, R, C, S>(f: F, prefix: C) -> Result<R>
where
    F: FnOnce() -> Result<R>,
    C: 'a + Fn() -> S,
    S: Into<String>,
{
    #[derive(Debug)]
    struct WrappedError {
        prefix: String,
        cause: Box<dyn std::error::Error>,
    }
    impl std::fmt::Display for WrappedError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}: {}", self.prefix, self.cause)
        }
    }
    impl std::error::Error for WrappedError {}

    match f() {
        Err(err) => Err(Box::new(WrappedError {
            prefix: prefix().into(),
            cause: err,
        })),
        Ok(result) => Ok(result),
    }
}
