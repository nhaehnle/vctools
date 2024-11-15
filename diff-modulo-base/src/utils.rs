// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

use crate::diff;

pub use vctools_utils::prelude::*;

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
