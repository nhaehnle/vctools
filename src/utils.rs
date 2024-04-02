// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

use crate::diff;

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

fn read_bytes_impl(path: &Path) -> Result<Vec<u8>> {
    try_forward(
        || -> Result<Vec<u8>> {
            let mut file = File::open(path)?;
            let mut buffer: Vec<u8> = Vec::new();
            file.read_to_end(&mut buffer)?;
            Ok(buffer)
        },
        || path.display().to_string(),
    )
}

pub fn read_bytes<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    read_bytes_impl(path.as_ref())
}

fn read_diff_impl(buffer: &mut diff::Buffer, path: &Path) -> Result<diff::Diff> {
    let buf = read_bytes_impl(path)?;
    try_forward(
        || -> Result<diff::Diff> {
            let range = buffer.insert(&buf)?;
            diff::Diff::parse(buffer, range)
        },
        || path.display().to_string(),
    )
}

pub fn read_diff<P: AsRef<Path>>(buffer: &mut diff::Buffer, path: P) -> Result<diff::Diff> {
    read_diff_impl(buffer, path.as_ref())
}

pub(crate) fn trim_ascii(mut s: &[u8]) -> &[u8] {
    while let Some((ch, tail)) = s.split_first() {
        if !ch.is_ascii_whitespace() {
            break;
        }
        s = tail;
    }

    while let Some((ch, head)) = s.split_last() {
        if !ch.is_ascii_whitespace() {
            break;
        }
        s = head;
    }

    s
}
