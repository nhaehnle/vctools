// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::diff;

pub use vctools_utils::files::read_bytes;
pub use vctools_utils::prelude::*;

fn read_diff_impl(buffer: &mut diff::Buffer, path: &Path) -> Result<diff::Diff> {
    let buf = read_bytes(path)?;
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
