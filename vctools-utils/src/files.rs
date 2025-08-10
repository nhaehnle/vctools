// SPDX-License-Identifier: MIT

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

use crate::prelude::*;

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
