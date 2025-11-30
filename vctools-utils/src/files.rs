// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::prelude::*;

fn read_bytes_impl(path: &Path) -> Result<Vec<u8>> {
    try_forward(
        || -> Result<Vec<u8>> { Ok(std::fs::read(path)?) },
        || path.display().to_string(),
    )
}

pub fn read_bytes<P: AsRef<Path>>(path: P) -> Result<Vec<u8>> {
    read_bytes_impl(path.as_ref())
}
