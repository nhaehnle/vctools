// SPDX-License-Identifier: MIT

///! File representation.
///!
///! Provides a representation of diff [`FileName`] and of file contents via [`File`]`
use std::{
    cell::RefCell,
    ops::{Bound, Range, RangeBounds},
};

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

pub fn parse_diff_path(
    path: BufferRef,
    strip_path_components: usize,
    buffer: &Buffer,
) -> Result<Option<BufferRef>> {
    if &buffer[path] == b"/dev/null" {
        return Ok(None);
    }

    if path.is_empty() {
        Err("empty diff file path")?;
    }

    try_forward(
        || -> Result<_> {
            let mut path_ref = path;
            let mut path = &buffer[path];
            if path[0] == b'/' {
                path = &path[1..];
                path_ref = path_ref.slice(1..);
            }

            for _ in 0..strip_path_components {
                let Some((idx, _)) = path.iter().find_position(|&b| *b == b'/') else {
                    return Err("path does not have enough components")?;
                };
                path = &path[idx + 1..];
                path_ref = path_ref.slice(idx + 1..);
            }

            Ok(Some(path_ref))
        },
        || String::from_utf8_lossy(&buffer[path]),
    )
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
    pub fn name_ref(&self) -> BufferRef {
        self.name
    }

    pub fn name<'slf, 'buf>(&'slf self, buffer: &'buf Buffer) -> &'buf [u8] {
        &buffer[self.name]
    }

    /// Return the number of lines in the file, if known.
    pub fn num_lines(&self) -> Option<u32> {
        if self.have_end_of_file {
            Some(self.parts.last().map(|p| p.lines.end).unwrap_or(0))
        } else {
            None
        }
    }

    fn lines_impl<'a>(
        &'a self,
        range: Range<u32>,
        buffer: &'a Buffer,
    ) -> impl ExactSizeIterator<Item = Option<BufferRef>> + 'a {
        struct LineIterator<'it> {
            buffer: &'it Buffer,
            file: &'it File,
            next: Landmark,
            end_line: u32,
        }
        impl<'it> Iterator for LineIterator<'it> {
            type Item = Option<BufferRef>;

            fn next(&mut self) -> Option<Self::Item> {
                if self.next.line >= self.end_line {
                    None
                } else {
                    if let Some(part) = self.file.parts.get(self.next.part as usize) {
                        if self.next.line >= part.lines.start {
                            if self.next.line + 1 < part.lines.end {
                                let text_ref = part.text.slice(self.next.offset as usize..);
                                let text = &self.buffer[text_ref];
                                let offset_nl =
                                    text.iter().find_position(|ch| **ch == b'\n').unwrap().0;
                                let line = text_ref.slice(..offset_nl + 1);
                                self.next.line += 1;
                                self.next.offset += (offset_nl + 1) as u32;
                                Some(Some(line))
                            } else {
                                // We're producing the last line of this part.
                                let line = part.text.slice(self.next.offset as usize..);
                                self.next.line += 1;
                                self.next.part += 1;
                                self.next.offset = 0;
                                Some(Some(line))
                            }
                        } else {
                            // We're in the unknown gap before the current part.
                            self.next.line += 1;
                            Some(None)
                        }
                    } else {
                        // We're past the end of the known lines.
                        self.next.line += 1;
                        Some(None)
                    }
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = (self.end_line - self.next.line) as usize;
                (len, Some(len))
            }
        }
        impl<'it> ExactSizeIterator for LineIterator<'it> {
            fn len(&self) -> usize {
                (self.end_line - self.next.line) as usize
            }
        }

        let (next, _) = self.find_line(range.start, buffer);

        LineIterator {
            buffer,
            file: self,
            next,
            end_line: range.end,
        }
    }

    pub fn lines<'a>(
        &'a self,
        range: impl RangeBounds<u32>,
        buffer: &'a Buffer,
    ) -> impl ExactSizeIterator<Item = Option<BufferRef>> + 'a {
        let start = match range.start_bound() {
            Bound::Unbounded => 0,
            Bound::Included(x) => *x,
            Bound::Excluded(x) => *x + 1,
        };
        let end = match range.end_bound() {
            Bound::Excluded(x) => *x,
            Bound::Included(x) => *x + 1,
            // Unbounded end may only be used with fully known files.
            Bound::Unbounded => self.num_lines().unwrap(),
        };

        self.lines_impl(start..end, buffer)
    }

    /// Returns the earliest possible landmark after or equal to the given line
    /// and, if the line is found and known and its end is trivial to determine,
    /// the end offset within the found part.
    fn find_line(&self, line: u32, buffer: &Buffer) -> (Landmark, Option<u32>) {
        // Find the neighboring landmarks that frame the searched-for line.
        let mut cache = self.landmark_lookup_cache.borrow_mut();
        let forward = cache.1.line <= line;
        let lm_idx_post = self.landmarks.partition_point_with_hint(
            if forward { cache.0 } else { cache.0 + 1 },
            forward,
            |lm| lm.line <= line,
        );
        if lm_idx_post == 0 {
            return (
                Landmark {
                    line,
                    part: 0,
                    offset: 0,
                },
                None,
            );
        }
        if lm_idx_post >= self.landmarks.len() {
            return (
                Landmark {
                    line,
                    part: self.parts.len() as u32,
                    offset: 0,
                },
                None,
            );
        }
        let lm_idx_pre = lm_idx_post - 1;

        // Refine the framing landmarks using the cache if possible.
        let mut lm_pre;
        let mut lm_post;
        if forward {
            lm_post = self.landmarks[lm_idx_post];
            if lm_idx_pre == cache.0 {
                lm_pre = cache.1;
                let pre_part = &self.parts[lm_pre.part as usize];
                if line < pre_part.lines.end {
                    lm_post.part = lm_pre.part;
                    lm_post.line = pre_part.lines.end;
                    lm_post.offset = pre_part.text.len() as u32;
                } else if lm_post.part - lm_pre.part >= 2 && lm_pre.offset != 0 {
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
                if let Some(post_part) = self
                    .parts
                    .get(lm_post.part as usize)
                    .filter(|p| p.lines.start <= line)
                {
                    lm_pre.part = lm_post.part;
                    lm_pre.line = post_part.lines.start;
                    lm_pre.offset = 0;
                } else if lm_post.part - lm_pre.part >= 2 && lm_post.offset != 0 {
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
                return (lm_post, None);
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
                return (lm_pre, Some(part.text.len() as u32));
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

        // Detect when the line is in the unknown gap between parts.
        let part = &self.parts[lm_pre.part as usize];
        if part.lines.end <= line {
            assert!(lm_pre.part + 1 == lm_post.part);

            *cache = (lm_idx_pre, lm_post);

            lm_post.line = line;
            return (lm_post, None);
        }

        // Find the target line using a linear scan.
        assert!(lm_pre.line <= line);

        while lm_pre.line != line {
            let text = &buffer[part.text.slice(lm_pre.offset as usize..)];
            let next_line_offset = text
                .iter()
                .enumerate()
                .find(|(_, ch)| **ch == b'\n')
                .unwrap()
                .0 as u32
                + 1;
            lm_pre.line += 1;
            lm_pre.offset += next_line_offset;
        }

        *cache = (lm_idx_pre, lm_pre);

        if lm_pre.part < lm_post.part {
            lm_post.part = lm_pre.part;
            lm_post.line = part.lines.end;
            lm_post.offset = part.text.len() as u32;
        }

        let end_offset = (line + 1 == lm_post.line).then_some(lm_post.offset);

        (lm_pre, end_offset)
    }

    /// Return the line with the given (0-based) number, or None if the line
    /// is not known.
    ///
    /// The returned range includes the final newline character, except for
    /// the last line of the file if the file does not end with a newline.
    pub fn line_ref(&self, line: u32, buffer: &Buffer) -> Option<BufferRef> {
        let (lm, end_offset) = self.find_line(line, buffer);
        if self
            .parts
            .get(lm.part as usize)
            .is_none_or(|part| lm.line < part.lines.start)
        {
            return None;
        }

        assert!(self.parts[lm.part as usize].lines.start <= line);
        assert!(line < self.parts[lm.part as usize].lines.end);

        let end_offset = end_offset.unwrap_or_else(|| {
            let part = &self.parts[lm.part as usize];
            let text = &buffer[part.text.slice(lm.offset as usize..)];
            let next_line_offset = text
                .iter()
                .enumerate()
                .find(|(_, ch)| **ch == b'\n')
                .unwrap()
                .0 as u32
                + 1;
            lm.offset + next_line_offset
        });

        return Some(
            self.parts[lm.part as usize]
                .text
                .slice(lm.offset as usize..end_offset as usize),
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

    fn push_text_impl(&mut self, buffer: &Buffer, lines: Range<u32>, text: BufferRef) {
        assert!(
            lines.start >= self.num_lines(),
            "lines must be inserted in order"
        );
        assert!(lines.start < lines.end);
        assert!(!self.must_be_eof(buffer));

        if let Some(last_part) = self
            .parts
            .last_mut()
            .filter(|p| p.lines.end == lines.start && p.text.end == text.begin)
        {
            // Extend the last part.
            last_part.lines.end = lines.end;
            last_part.text.end = text.end;
        } else {
            self.parts.push(Part { lines, text });
        }
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

        self.push_text_impl(buffer, line..line_end, text);

        Ok(())
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
        self.push_text_impl(buffer, line..line + 1, text);
        Ok(())
    }

    /// Copy a contiguous range of known lines from `src_file` into `self`.
    ///
    /// The lines are taken from the given range of `src_lines`. As many as
    /// possible (zero or more) known lines are taken from the start of the range.
    ///
    /// The lines are inserted starting at (0-based) line number `dst_line`.
    ///
    /// On success, the function returns `(end_dst_line, remaining_range)`.
    /// The end_dst_line indicates the next line to be added to the file.
    /// If the remaining range is non-empty, it starts at the next known line
    /// after the contiguous lines that were copied.
    pub fn copy_known_lines(
        &mut self,
        mut dst_line: u32,
        src_file: &File,
        mut src_lines: Range<u32>,
        buffer: &Buffer,
    ) -> Result<(u32, Range<u32>)> {
        let (mut lm, mut maybe_end_offset) = src_file.find_line(src_lines.start, buffer);

        while src_lines.len() != 0 {
            if lm.part as usize >= src_file.parts.len() {
                src_lines.start = src_lines.end;
                break;
            }

            let lm_part = &src_file.parts[lm.part as usize];
            if src_lines.start < lm_part.lines.start {
                src_lines.start = std::cmp::min(src_lines.end, lm_part.lines.start);
                break;
            }

            let num_lines =
                std::cmp::min(lm_part.lines.end - src_lines.start, src_lines.len() as u32);
            let end_line = dst_line
                .checked_add(num_lines)
                .ok_or("Files with more than 2**32 - 1 lines are unsupported")?;

            let end_offset =
                maybe_end_offset
                    .take()
                    .filter(|_| src_lines.len() == 1)
                    .unwrap_or_else(|| {
                        if lm_part.lines.end <= src_lines.end {
                            lm_part.text.len() as u32
                        } else {
                            let (lm_end, _) = src_file.find_line(src_lines.end, buffer);
                            lm_end.offset
                        }
                    });
            let text_ref = lm_part.text.slice(lm.offset as usize..end_offset as usize);
            self.push_text_impl(buffer, dst_line..end_line, text_ref);
            src_lines.start += num_lines;
            dst_line += num_lines;

            lm.part += 1;
            lm.offset = 0;
        }

        Ok((dst_line, src_lines))
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

        let mut lines = file.lines(0..7, &buffer);
        assert_eq!(lines.len(), 7);
        assert_eq!(&buffer[lines.next().unwrap().unwrap()], b"first line\n");
        assert!(lines.next().unwrap().is_none());
        assert!(lines.next().unwrap().is_none());
        assert!(lines.next().unwrap().is_none());
        assert!(lines.next().unwrap().is_none());
        assert_eq!(&buffer[lines.next().unwrap().unwrap()], b"other line\n");
        assert!(lines.next().unwrap().is_none());
        assert!(lines.next().is_none());

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

        {
            let mut lines = file.lines(7..13, &buffer);
            assert_eq!(lines.len(), 6);
            assert_eq!(&buffer[lines.next().unwrap().unwrap()], b"line 7\n");
            assert!(lines.next().unwrap().is_none());
            assert!(lines.next().unwrap().is_none());
            assert_eq!(&buffer[lines.next().unwrap().unwrap()], b"line 10\n");
            assert_eq!(&buffer[lines.next().unwrap().unwrap()], b"line 11\n");
            assert_eq!(&buffer[lines.next().unwrap().unwrap()], b"line 12\n");
            assert!(lines.next().is_none());
        }

        let mut builder = FileBuilder::new();
        assert_eq!(
            builder.copy_known_lines(0, &file, 6..7, &buffer)?,
            (1, 7..7)
        );
        assert_eq!(
            builder.copy_known_lines(1, &file, 7..11, &buffer)?,
            (2, 10..11)
        );
        assert_eq!(
            builder.copy_known_lines(2, &file, 7..8, &buffer)?,
            (3, 8..8)
        );
        assert_eq!(
            builder.copy_known_lines(3, &file, 7..9, &buffer)?,
            (4, 9..9)
        );
        assert_eq!(
            builder.copy_known_lines(4, &file, 10..13, &buffer)?,
            (7, 13..13)
        );
        let file2 = builder.build(buffer.insert(b"filename2")?, true, &buffer);

        assert_eq!(file2.num_lines(), Some(7));
        assert_eq!(file2.line(0, &buffer).unwrap(), b"line 6\n");
        assert_eq!(file2.line(1, &buffer).unwrap(), b"line 7\n");
        assert_eq!(file2.line(2, &buffer).unwrap(), b"line 7\n");
        assert_eq!(file2.line(3, &buffer).unwrap(), b"line 7\n");
        assert_eq!(file2.line(4, &buffer).unwrap(), b"line 10\n");
        assert_eq!(file2.line(5, &buffer).unwrap(), b"line 11\n");
        assert_eq!(file2.line(6, &buffer).unwrap(), b"line 12\n");

        Ok(())
    }
}
