// SPDX-License-Identifier: MIT

///! A hunk is a contiguous block of changed and unchanged lines in a diff.
///!
///! The [`hunkify`] function can be used to produce hunks from a [`FileMatch`].

use super::{Buffer, FileMatch, MatchStatus, MatchStatusMarker, render};

#[derive(Debug, Clone, Copy)]
pub enum HunkLineStatus {
    Unchanged,
    Old(bool), // "unimportant" boolean
    New(bool), // "unimportant" boolean
}
impl HunkLineStatus {
    pub fn symbol_byte(self) -> u8 {
        match self {
            HunkLineStatus::Unchanged => b' ',
            HunkLineStatus::Old(false) => b'-',
            HunkLineStatus::New(false) => b'+',
            HunkLineStatus::Old(true) => b'<',
            HunkLineStatus::New(true) => b'>',
        }
    }

    pub fn covers_old(self) -> bool {
        match self {
            HunkLineStatus::Unchanged | HunkLineStatus::Old(_) => true,
            HunkLineStatus::New(_) => false,
        }
    }

    pub fn covers_new(self) -> bool {
        match self {
            HunkLineStatus::Unchanged | HunkLineStatus::New(_) => true,
            HunkLineStatus::Old(_) => false,
        }
    }

    pub fn important(self) -> bool {
        match self {
            HunkLineStatus::Unchanged => false,
            HunkLineStatus::Old(unimportant) | HunkLineStatus::New(unimportant) => !unimportant,
        }
    }

    fn counts<I>(iter: I) -> (u32, u32)
    where
        I: Iterator<Item = HunkLineStatus>,
    {
        let mut old_count = 0;
        let mut new_count = 0;
        for status in iter {
            if status.covers_old() {
                old_count += 1;
            }
            if status.covers_new() {
                new_count += 1;
            }
        }
        (old_count, new_count)
    }
}

#[derive(Debug, Clone)]
pub struct HunkLine {
    pub status: HunkLineStatus,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_begin: u32, // 1-based line numbers
    pub new_begin: u32, // 1-based line numbers
    pub lines: Vec<HunkLine>,
}
impl Hunk {
    /// Returns line counts (num_old_lines, num_new_lines) for the hunk.
    ///
    /// Unchanged lines count towards both old and new line counts.
    pub fn counts(&self) -> (u32, u32) {
        HunkLineStatus::counts(self.lines.iter().map(|line| line.status))
    }

    pub fn render(&self, header: bool, writer: &mut dyn render::ChunkWriter) {
        if header {
            // TODO: Correct hunk header when one of old/new is an empty file
            let (old_count, new_count) = self.counts();
            writer.push_chunk(render::Chunk {
                context: render::Context::Unknown,
                contents: render::ChunkContents::HunkHeader {
                    old_begin: self.old_begin,
                    old_count,
                    new_begin: self.new_begin,
                    new_count,
                },
            });
        }

        for line in &self.lines {
            writer.push_chunk(render::Chunk {
                context: render::Context::Unknown,
                contents: render::ChunkContents::Line { line: line.clone() },
            });
        }
    }
}

/// Iterate over hunks of the diff appropriate for rendering.
///
/// If `num_context_lines` is `None`, iterate over hunks covering all known
/// lines in the diff, regardless of whether they are changed or not.
///
/// Otherwise, hunks will be reduced to at most the given number of lines
/// surrounding important changes.
pub fn hunkify<'a>(
    file_match: &'a FileMatch,
    num_context_lines: Option<usize>,
    buffer: &'a Buffer,
) -> impl Iterator<Item = Hunk> + 'a {
    Hunkify::new(file_match, num_context_lines, buffer)
}

#[derive(Debug, Clone, Copy)]
enum HunkLineRef {
    Old(u32, bool), // 0-based line number and "unimportant" bool
    New(u32, bool), // 0-based line number and "unimportant" bool
    Unchanged(u32, u32),
}

#[derive(Debug)]
struct HunkLineRefIter<'a> {
    file_match: &'a FileMatch,
    sm: MatchStatusMarker,
    next_idx: usize,
}
impl HunkLineRefIter<'_> {
    fn new(file_match: &FileMatch) -> HunkLineRefIter<'_> {
        let mut slf = HunkLineRefIter {
            file_match,
            sm: file_match.status_markers[0],
            next_idx: 1,
        };
        assert!(slf.sm.old_line == 0 && slf.sm.new_line == 0);
        slf.advance_idx();
        slf
    }

    fn is_end(&self) -> bool {
        self.next_idx >= self.file_match.status_markers.len()
    }

    fn advance_idx(&mut self) {
        while !self.is_end() {
            let sm_next = &self.file_match.status_markers[self.next_idx];
            if self.sm.old_line < sm_next.old_line || self.sm.new_line < sm_next.new_line {
                break;
            }
            assert!(self.sm.old_line == sm_next.old_line);
            assert!(self.sm.new_line == sm_next.new_line);
            self.sm = *sm_next;
            self.next_idx += 1;
        }
    }

    /// Fast-forward to the earliest important change starting at the current position.
    ///
    /// This is a no-op if the current position already points to an important change.
    ///
    /// Returns `false` if the end of the file match is reached without seeing an important change.
    fn fast_forward_to_important_change(&mut self) -> bool {
        while !matches!(self.sm.status, MatchStatus::Changed { unimportant: false }) {
            if self.is_end() {
                return false;
            }
            self.sm = self.file_match.status_markers[self.next_idx];
            self.next_idx += 1;
            self.advance_idx();
        }
        true
    }

    /// Rewind by the given number of lines.
    ///
    /// Setting `by` to 0 is a no-op.
    ///
    /// Never rewinds past the given (old_line, new_line) bound.
    ///
    /// Returns the number of lines not rewound, i.e. returns non-zero if
    /// the bound is reached before rewinding is complete.
    fn rewind(&mut self, mut by: usize, bound: (u32, u32)) -> usize {
        'outer: while by != 0 && (self.sm.old_line > bound.0 || self.sm.new_line > bound.1) {
            let mut sm_prev = &self.file_match.status_markers[self.next_idx - 1];
            assert!(sm_prev.status == self.sm.status);
            assert!(sm_prev.old_line <= self.sm.old_line);
            assert!(sm_prev.new_line <= self.sm.new_line);

            while self.sm.old_line == sm_prev.old_line && self.sm.new_line == sm_prev.new_line {
                if self.next_idx == 1 {
                    assert!(self.sm.old_line == 0 && self.sm.new_line == 0);
                    break 'outer;
                }

                self.next_idx -= 1;

                sm_prev = &self.file_match.status_markers[self.next_idx - 1];
                self.sm.status = sm_prev.status;
            }

            let old_delta_bound = (self.sm.old_line - bound.0) as usize;
            let new_delta_bound = (self.sm.new_line - bound.1) as usize;
            match self.sm.status {
                MatchStatus::Unchanged => {
                    let range = ((self.sm.old_line - sm_prev.old_line) as usize)
                        .min(old_delta_bound);
                    assert!(range == ((self.sm.new_line - sm_prev.new_line) as usize)
                        .min(new_delta_bound));
                    let delta = by.min(range);
                    self.sm.old_line -= delta as u32;
                    self.sm.new_line -= delta as u32;
                    by -= delta;
                }
                MatchStatus::Changed { .. } => {
                    let range_old = (self.sm.old_line - sm_prev.old_line) as usize;
                    let range_new = (self.sm.new_line - sm_prev.new_line) as usize;
                    assert!(range_old == 0 || old_delta_bound == 0 || new_delta_bound >= range_new);

                    let delta = by.min(range_new).min(new_delta_bound);
                    self.sm.new_line -= delta as u32;
                    by -= delta;
                                      
                    let delta = by.min(range_old).min(old_delta_bound);
                    self.sm.old_line -= delta as u32;
                    by -= delta;
                }
            }
        }
        by
    }

    fn current(&self) -> (u32, u32) {
        (self.sm.old_line, self.sm.new_line)
    }
}
impl Iterator for HunkLineRefIter<'_> {
    type Item = HunkLineRef;

    fn next(&mut self) -> Option<HunkLineRef> {
        let out;
        match self.sm.status {
            MatchStatus::Unchanged => {
                out = HunkLineRef::Unchanged(self.sm.old_line, self.sm.new_line);
                self.sm.old_line += 1;
                self.sm.new_line += 1;
            },
            MatchStatus::Changed { unimportant } => {
                let sm_next = &self.file_match.status_markers[self.next_idx];
                if self.sm.old_line < sm_next.old_line {
                    out = HunkLineRef::Old(self.sm.old_line, unimportant);
                    self.sm.old_line += 1;
                } else {
                    assert!(self.sm.new_line < sm_next.new_line);
                    out = HunkLineRef::New(self.sm.new_line, unimportant);
                    self.sm.new_line += 1;
                }
            },
        };
        self.advance_idx();
        Some(out)
    }
}

#[derive(Debug)]
struct Hunkify<'a> {
    buffer: &'a Buffer,
    num_context_lines: Option<usize>,
    iter: HunkLineRefIter<'a>,
}
impl<'a> Hunkify<'a> {
    fn new(file_match: &'a FileMatch, num_context_lines: Option<usize>, buffer: &'a Buffer) -> Hunkify<'a> {
        Hunkify {
            buffer,
            num_context_lines,
            iter: HunkLineRefIter::new(file_match),
        }
    }
}
impl<'a> Iterator for Hunkify<'a> {
    type Item = Hunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iter.is_end() && (self.num_context_lines.is_some() || self.iter.sm.old_line != 0 || self.iter.sm.new_line != 0) {
            // Fuse the iterator.
            return None;
        }

        if let Some(num_context_lines) = self.num_context_lines {
            let bound = self.iter.current();
            if !self.iter.fast_forward_to_important_change() {
                return None;
            }
            self.iter.rewind(num_context_lines, bound);
        }

        let mut hunk = Hunk {
            old_begin: self.iter.sm.old_line + 1,
            new_begin: self.iter.sm.new_line + 1,
            lines: Vec::new(),
        };
        let mut unimportant_tail = 0;
        let mut seen_important = false;

        while self.num_context_lines.is_none_or(|ncl| unimportant_tail / 2 <= ncl) {
            let line_ref = match self.iter.next() {
                Some(lr) => lr,
                None => break,
            };

            let (line, status) = match line_ref {
                HunkLineRef::Unchanged(old_line, _new_line) => {
                    let line = self.iter.file_match.old.as_ref().and_then(|old| old.line(old_line, self.buffer));
                    (line, HunkLineStatus::Unchanged)
                }
                HunkLineRef::Old(old_line, unimportant) => {
                    let line = self.iter.file_match.old.as_ref().and_then(|old| old.line(old_line, self.buffer));
                    (line, HunkLineStatus::Old(unimportant))
                }
                HunkLineRef::New(new_line, unimportant) => {
                    let line = self.iter.file_match.new.as_ref().and_then(|new| new.line(new_line, self.buffer));
                    (line, HunkLineStatus::New(unimportant))
                }
            };

            let Some(line) = line else {
                if seen_important {
                    break;
                }

                // Unknown line at the start of the hunk. Reset the hunk and
                // continue scanning forward until a known line appears.
                hunk = Hunk {
                    old_begin: self.iter.sm.old_line + 1,
                    new_begin: self.iter.sm.new_line + 1,
                    lines: Vec::new(),
                };
                unimportant_tail = 0;
                continue;
            };

            if status.important() {
                unimportant_tail = 0;
                seen_important = true;
            } else {
                unimportant_tail += 1;
            }
            hunk.lines.push(HunkLine {
                status,
                contents: line.to_vec(),
            });
        }

        if let Some(ncl) = self.num_context_lines {
            if unimportant_tail > ncl {
                let num_trunc = unimportant_tail - ncl;
                hunk.lines.truncate(hunk.lines.len() - num_trunc);
                self.iter.rewind(num_trunc, (0, 0));
            }
        } else if hunk.lines.is_empty() {
            return None;
        }

        Some(hunk)
    }
}
