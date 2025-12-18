// SPDX-License-Identifier: MIT

///! File representation.
///!
///! Provides a representation of diff [`FileName`] and of file contents via [`File`]`

use std::{cell::RefCell, ops::Range};

use itertools::Itertools;
use vctools_utils::prelude::*;

use super::buffer::{Buffer, BufferRef};

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

/// Part of a file covering a range of lines.
///
/// The text reference includes the final newline character except when the
/// part covers the end of the file and the file does not end in a newline
/// character.
#[derive(Debug, Clone)]
struct Part {
    lines: Range<u32>,
    text: BufferRef,
}

/// A landmark indicates where a given line starts in the file representation.
#[derive(Debug, Clone, Copy)]
struct Landmark {
    line: u32,
    part: u32,
    offset: u32,
}

/// A file with name and contents.
///
/// The file contents may only be partially known. Gaps in file knowledge are at
/// a line granularity: each line of the file is either fully known or fully
/// unknown.
///
/// [`File`]s are created using [`FileBuilder`].
#[derive(Debug, Clone)]
pub struct File {
    name: BufferRef,
    parts: Vec<Part>,
    have_end_of_file: bool,

    /// Landmarks for fast binary search by line number. The first landmark
    /// is always at the start of the first part, and the last landmark always
    /// corresponds to the end of the last part.
    landmarks: Vec<Landmark>,
    landmark_lookup_cache: RefCell<(usize, Landmark)>,
}
impl File {
    /// Return the number of lines in the file, if known.
    pub fn num_lines(&self) -> Option<u32> {
        if self.have_end_of_file {
            Some(self.parts.last().map(|p| p.lines.end).unwrap_or(0))
        } else {
            None
        }
    }

    /// Return the line with the given (0-based) number, or None if the line
    /// is not known.
    ///
    /// The returned range includes the final newline character, except for
    /// the last line of the file if the file does not end with a newline.
    pub fn line_ref(&self, line: u32, buffer: &Buffer) -> Option<BufferRef> {
        // Find the neighboring landmarks that frame the searched-for line.
        let mut cache = self.landmark_lookup_cache.borrow_mut();
        let forward = cache.1.line <= line;
        let lm_idx_post = self.landmarks.partition_point_with_hint(
            if forward { cache.0 } else { cache.0 + 1 },
            forward,
            |lm| lm.line <= line,
        );
        if lm_idx_post == 0 || lm_idx_post >= self.landmarks.len() {
            return None;
        }
        let lm_idx_pre = lm_idx_post - 1;

        // Refine the framing landmarks using the cache if possible.
        let mut lm_pre;
        let mut lm_post;
        if forward {
            lm_post = self.landmarks[lm_idx_post];
            if lm_idx_pre == cache.0 {
                lm_pre = cache.1;
                if lm_post.part - lm_pre.part >= 2 && lm_pre.offset != 0 {
                    lm_pre.part += 1;
                    lm_pre.line = self.parts[lm_pre.part as usize].lines.start;
                    lm_pre.offset = 0;
                }
            } else {
                lm_pre = self.landmarks[lm_idx_pre];
            };
        } else {
            lm_pre = self.landmarks[lm_idx_pre];
            if lm_idx_pre == cache.0 {
                lm_post = cache.1;
                if lm_post.part - lm_pre.part >= 2 && lm_post.offset != 0 {
                    lm_post.line = self.parts[lm_post.part as usize].lines.start;
                    lm_post.offset = 0;
                }
            } else {
                lm_post = self.landmarks[lm_idx_post];
            }
        }

        // Narrow the range down to a single part.
        //
        // We ultimately want to do a binary search over parts, but instead
        // of starting with the entire landmark-based range, we bias our
        // search towards the cached point.
        if lm_post.part - lm_pre.part >= 2 {
            assert!(lm_pre.offset == 0);
            assert!(lm_post.offset == 0);

            if forward {
                let mut step = 1;
                while step < lm_post.part - lm_pre.part {
                    let part = &self.parts[(lm_pre.part + step) as usize];
                    if line < part.lines.start {
                        lm_post.part = lm_pre.part + step;
                        lm_post.line = part.lines.start;
                        break;
                    }
                    lm_pre.part = lm_pre.part + step;
                    lm_pre.line = part.lines.start;
                    step *= 2;
                }
            } else {
                let mut step = 1;
                while step < lm_post.part - lm_pre.part {
                    let part = &self.parts[(lm_post.part - step) as usize];
                    if part.lines.start <= line {
                        lm_pre.part = lm_post.part - step;
                        lm_pre.line = part.lines.start;
                        break;
                    }
                    lm_post.part = lm_post.part - step;
                    lm_post.line = part.lines.start;
                    step *= 2;
                }
            }
        }

        // Now do the binary search over parts.
        while lm_post.part - lm_pre.part >= 2 {
            let end_line = self.parts[(lm_post.part - 1) as usize].lines.end;
            if end_line <= line {
                *cache = (lm_idx_pre, lm_post);
                return None;
            }

            assert!(end_line - lm_pre.line >= lm_post.part - lm_pre.part);
            if end_line - lm_pre.line == lm_post.part - lm_pre.part {
                // Handle the case where we've narrowed it down to
                // a run of single-line parts.
                let delta = line - lm_pre.line;
                lm_pre.part += delta;
                lm_pre.line += delta;

                let part = &self.parts[lm_pre.part as usize];
                *cache = (lm_idx_pre, lm_pre);
                return Some(part.text);
            }

            let mid = lm_pre.part + (lm_post.part - lm_pre.part) / 2;
            if line >= self.parts[mid as usize].lines.start {
                lm_pre.part = mid;
                lm_pre.line = self.parts[mid as usize].lines.start;
            } else {
                lm_post.part = mid;
                lm_post.line = self.parts[mid as usize].lines.start;
            }
        }

        // Normalize to a single part.
        let part = &self.parts[lm_pre.part as usize];
        if lm_pre.part < lm_post.part {
            lm_post.part = lm_pre.part;
            lm_post.line = part.lines.end;
            lm_post.offset = part.text.len() as u32;

            if lm_post.line <= line {
                *cache = (lm_idx_pre, lm_pre);
                return None;
            }
        }

        // Find the target line using a linear scan.
        assert!(lm_pre.line < lm_post.line);

        if lm_post.line - lm_pre.line > 1 {
            loop {
                let text = &buffer[part
                    .text
                    .slice(lm_pre.offset as usize..lm_post.offset as usize)];
                let next_line_offset = text
                    .iter()
                    .enumerate()
                    .find(|(_, ch)| **ch == b'\n')
                    .unwrap()
                    .0 as u32 + 1;
                if line == lm_pre.line {
                    lm_post.line = lm_pre.line + 1;
                    lm_post.offset = lm_pre.offset + next_line_offset;
                    break;
                }
                lm_pre.line += 1;
                lm_pre.offset += next_line_offset;
            }
        }

        *cache = (lm_idx_pre, lm_pre);
        return Some(
            part.text
                .slice(lm_pre.offset as usize..lm_post.offset as usize),
        );
    }

    /// Return the line with the given (0-based) number, or None if the line
    /// is not known.
    ///
    /// The returned slice includes the final newline character, except for
    /// the last line of the file if the file does not end with a newline.
    pub fn line<'slf, 'buf>(&'slf self, line: u32, buffer: &'buf Buffer) -> Option<&'buf [u8]> {
        self.line_ref(line, buffer).map(|r| &buffer[r])
    }
}

#[derive(Debug, Default)]
pub struct FileBuilder {
    parts: Vec<Part>,
}
impl FileBuilder {
    pub fn new() -> FileBuilder {
        Self::default()
    }

    pub fn build(self, name: BufferRef, have_end_of_file: bool, buffer: &Buffer) -> File {
        assert!(!self.must_be_eof(buffer) || have_end_of_file);
        assert!(self.parts.len() < u32::MAX as usize);

        let mut landmarks: Vec<Landmark> = vec![];

        for (part_idx, part) in self.parts.iter().enumerate() {
            const LANDMARK_SPACING: u32 = 512;
            let is_reset = landmarks.last().is_none_or(|lm| lm.offset != 0);
            let is_large = part.lines.len() > 1 && part.text.len() > LANDMARK_SPACING as usize;

            if !is_reset && !is_large {
                continue;
            }

            landmarks.push(Landmark {
                line: part.lines.start,
                part: part_idx as u32,
                offset: 0,
            });

            if !is_large {
                continue;
            }

            // Iterate over the starts of newlines (except at the very end)
            // and insert landmarks roughly every LANDMARK_SPACING bytes.
            for ((old_line, old_idx), (new_line, new_idx)) in std::iter::once((0, 0))
                .chain(
                    buffer[part.text.slice(0..part.text.len() - 1)]
                        .iter()
                        .enumerate()
                        .filter(|(_, ch)| **ch == b'\n')
                        .scan(0u32, |line, (idx, _)| {
                            *line += 1;
                            Some((*line, (idx + 1) as u32))
                        }),
                )
                .tuple_windows()
            {
                if new_idx - old_idx > LANDMARK_SPACING
                    && old_idx != landmarks.last().unwrap().offset
                {
                    landmarks.push(Landmark {
                        line: part.lines.start + old_line,
                        part: part_idx as u32,
                        offset: old_idx as u32,
                    });
                }

                if new_idx - landmarks.last().unwrap().offset > LANDMARK_SPACING {
                    landmarks.push(Landmark {
                        line: part.lines.start + new_line,
                        part: part_idx as u32,
                        offset: new_idx as u32,
                    });
                }
            }
        }

        landmarks.push(Landmark {
            line: self.num_lines(),
            part: self.parts.len() as u32,
            offset: 0,
        });

        File {
            name,
            parts: self.parts,
            have_end_of_file,
            landmark_lookup_cache: RefCell::new((0, landmarks[0])),
            landmarks,
        }
    }

    fn num_lines(&self) -> u32 {
        self.parts.last().map(|p| p.lines.end).unwrap_or(0)
    }

    fn must_be_eof(&self, buffer: &Buffer) -> bool {
        self.parts
            .last()
            .is_some_and(|p| !buffer[p.text].ends_with(b"\n"))
    }

    fn push_text_impl(
        &mut self,
        buffer: &Buffer,
        lines: Range<u32>,
        text: BufferRef,
    ) -> Result<()> {
        assert!(
            lines.start >= self.num_lines(),
            "lines must be inserted in order"
        );
        assert!(lines.start < lines.end);
        assert!(!self.must_be_eof(buffer));

        self.parts.push(Part { lines, text });
        Ok(())
    }

    pub fn push_text(&mut self, line: u32, text: BufferRef, buffer: &Buffer) -> Result<()> {
        let num_lines = buffer[text]
            .split_last()
            .map(|(_, s)| s)
            .unwrap_or(&[])
            .iter()
            .filter(|ch| **ch == b'\n')
            .count() as u32
            + 1;

        let line_end = line
            .checked_add(num_lines)
            .ok_or("Files with more than 2**32 - 1 lines are unsupported")?;

        self.push_text_impl(buffer, line..line_end, text)
    }

    pub fn push_line(&mut self, line: u32, text: BufferRef, buffer: &Buffer) -> Result<()> {
        if line == u32::MAX {
            Err("Files with more than 2**32 - 1 lines are unsupported")?
        }

        assert!(buffer[text]
            .iter()
            .enumerate()
            .find(|(_, ch)| **ch == b'\n')
            .is_none_or(|(idx, _)| idx == text.len() - 1));
        self.push_text_impl(buffer, line..line + 1, text)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_empty() -> Result<()> {
        let mut buffer = Buffer::new();
        let builder = FileBuilder::new();
        let file = builder.build(buffer.insert(b"filename")?, true, &buffer);

        assert_eq!(file.num_lines(), Some(0));
        assert_eq!(file.line(0, &buffer), None);
        assert_eq!(file.line(42, &buffer), None);

        Ok(())
    }

    #[test]
    fn test_basic() -> Result<()> {
        let mut buffer = Buffer::new();
        let mut builder = FileBuilder::new();
        builder.push_line(0, buffer.insert(b"first line\n")?, &buffer)?;
        builder.push_line(5, buffer.insert(b"other line\n")?, &buffer)?;
        let file = builder.build(buffer.insert(b"filename")?, false, &buffer);

        assert_eq!(file.num_lines(), None);
        assert_eq!(file.line(0, &buffer).unwrap(), b"first line\n");
        assert_eq!(file.line(5, &buffer).unwrap(), b"other line\n");
        assert_eq!(file.line(1, &buffer), None);
        assert_eq!(file.line(4, &buffer), None);
        assert_eq!(file.line(6, &buffer), None);
        assert_eq!(file.line(999, &buffer), None);
        assert_eq!(&buffer[file.line_ref(0, &buffer).unwrap()], b"first line\n");

        Ok(())
    }

    #[test]
    fn test_multiline() -> Result<()> {
        let mut buffer = Buffer::new();
        let mut builder = FileBuilder::new();
        builder.push_text(5, buffer.insert(b"line 5\nline 6\nline 7\n")?, &buffer)?;
        builder.push_text(10, buffer.insert(b"line 10\nline 11\n")?, &buffer)?;
        builder.push_text(12, buffer.insert(b"line 12\nline 13\n")?, &buffer)?;
        let file = builder.build(buffer.insert(b"filename")?, true, &buffer);

        assert_eq!(file.num_lines(), Some(14));
        assert_eq!(file.line(3, &buffer), None);
        assert_eq!(file.line(5, &buffer).unwrap(), b"line 5\n");
        assert_eq!(file.line(7, &buffer).unwrap(), b"line 7\n");
        assert_eq!(file.line(10, &buffer).unwrap(), b"line 10\n");
        assert_eq!(file.line(11, &buffer).unwrap(), b"line 11\n");
        assert_eq!(file.line(12, &buffer).unwrap(), b"line 12\n");
        assert_eq!(file.line(13, &buffer).unwrap(), b"line 13\n");
        assert_eq!(file.line(8, &buffer), None);
        assert_eq!(file.line(6, &buffer).unwrap(), b"line 6\n");

        Ok(())
    }
}
