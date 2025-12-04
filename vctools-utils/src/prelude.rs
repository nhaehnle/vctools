// SPDX-License-Identifier: MIT

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;
pub trait ResultExt<T> {
    fn as_ref_ok(&self) -> Result<&T>;
    fn as_mut_ok(&mut self) -> Result<&mut T>;
}
impl<T> ResultExt<T> for Result<T> {
    fn as_ref_ok(&self) -> Result<&T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => Err(err.to_string())?,
        }
    }

    fn as_mut_ok(&mut self) -> Result<&mut T> {
        match self {
            Ok(value) => Ok(value),
            Err(err) => Err(err.to_string())?,
        }
    }
}

pub fn err_from_str(msg: &str) -> Box<dyn std::error::Error + Send + Sync> {
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
        cause: Box<dyn std::error::Error + Send + Sync>,
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

#[derive(Debug, Clone)]
pub enum GCow<'a, T> {
    Borrowed(&'a T),
    Owned(T),
}
impl<'a, T> std::ops::Deref for GCow<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            GCow::Borrowed(value) => value,
            GCow::Owned(value) => value,
        }
    }
}
impl<'a, T: Clone> GCow<'a, T> {
    pub fn into_owned(self) -> T {
        match self {
            GCow::Borrowed(value) => value.clone(),
            GCow::Owned(value) => value,
        }
    }
}
impl<'a, T> From<&'a T> for GCow<'a, T> {
    fn from(value: &'a T) -> Self {
        GCow::Borrowed(value)
    }
}
impl<'a, T> From<T> for GCow<'a, T> {
    fn from(value: T) -> Self {
        GCow::Owned(value)
    }
}

pub use crate::partition_point::PartitionPointExt;
