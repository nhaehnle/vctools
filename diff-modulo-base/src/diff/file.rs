// SPDX-License-Identifier: MIT

use vctools_utils::prelude::*;

/// Represent an effective filename in a diff (without any prefix path
/// components). Missing means that the file is missing on the corresponding
/// side of the diff.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileName {
    Missing,
    Name(Vec<u8>),
}
impl Default for FileName {
    fn default() -> Self {
        FileName::Missing
    }
}
impl FileName {
    pub fn from_bytes(path: &[u8], strip_path_components: usize) -> Result<FileName> {
        if path == b"/dev/null" {
            return Ok(Self::Missing);
        }

        if path.is_empty() {
            return Err("empty diff file path".into());
        }

        try_forward(
            || -> Result<_> {
                let mut path = path;
                if path[0] == b'/' {
                    path = &path[1..];
                }

                for _ in 0..strip_path_components {
                    path = match path
                        .iter()
                        .enumerate()
                        .find(|(_, &b)| b == b'/')
                        .map(|(idx, _)| idx)
                    {
                        Some(idx) => &path[idx + 1..],
                        None => {
                            return Err("path does not have enough components".into());
                        }
                    };
                }

                Ok(Self::Name(path.into()))
            },
            || String::from_utf8_lossy(path),
        )
    }
}
