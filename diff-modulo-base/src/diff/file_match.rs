// SPDX-License-Identifier: MIT

use std::ops::Range;

use super::{Buffer, File, FileName, hunkify, render};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStatus {
    Unchanged,
    Changed {
        unimportant: bool,
    },
}
impl MatchStatus {
    pub fn is_changed(self) -> bool {
        matches!(self, MatchStatus::Changed { .. })
    }

    pub fn merge(self, other: MatchStatus) -> MatchStatus {
        match (self, other) {
            (MatchStatus::Unchanged, MatchStatus::Unchanged) => MatchStatus::Unchanged,
            (MatchStatus::Unchanged, MatchStatus::Changed { unimportant }) => {
                MatchStatus::Changed { unimportant }
            }
            (MatchStatus::Changed { unimportant }, MatchStatus::Unchanged) => {
                MatchStatus::Changed { unimportant }
            }
            (MatchStatus::Changed { unimportant: u1 }, MatchStatus::Changed { unimportant: u2 }) => {
                MatchStatus::Changed {
                    unimportant: u1 && u2,
                }
            }
        }
    }
}

/// Mark the beginning of a region with a given status (unchanged vs. changed).
/// 
/// Line numbers are 0-based.
#[derive(Debug, Clone, Copy)]
pub struct MatchStatusMarker {
    pub old_line: u32,
    pub new_line: u32,
    pub status: MatchStatus,
}
impl MatchStatusMarker {
    pub fn is_changed(&self) -> bool {
        self.status.is_changed()
    }
}

/// Describes how one version of a file matches up to another version of a file.
#[derive(Debug, Clone)]
pub struct FileMatch {
    // Old path as found in the "---" line
    pub old_path: Vec<u8>,

    /// New path as found in the "+++" line
    pub new_path: Vec<u8>,

    pub(crate) old: Option<File>,
    pub(crate) new: Option<File>,

    /// Status markers. Always contains an entry for the beginning, i.e.
    /// line numbers 0. The last entry is always of unchanged status and
    /// conceptually covers arbitrarily high line numbers.
    pub(crate) status_markers: Vec<MatchStatusMarker>,
}
impl FileMatch {
    pub fn render_header(&self, buffer: &Buffer, writer: &mut dyn render::ChunkWriter) {
        writer.push_chunk(render::Chunk {
            context: render::Context::Unknown,
            contents: render::ChunkContents::FileHeader {
                old_path: self.old_path.clone(),
                old_name: self.old.as_ref().map(|old| FileName::Name(old.name(buffer).to_vec())).unwrap_or(FileName::Missing),
                new_path: self.new_path.clone(),
                new_name: self.new.as_ref().map(|new| FileName::Name(new.name(buffer).to_vec())).unwrap_or(FileName::Missing),
            },
        });
    }

    pub fn render(&self, buffer: &Buffer, num_context_lines: usize, writer: &mut dyn render::ChunkWriter) {
        let mut printed_header = false;
        for hunk in hunkify(self, Some(num_context_lines), buffer) {
            if !printed_header {
                self.render_header(buffer, writer);
                printed_header = true;
            }
            hunk.render(true, writer);
        }
    }

    pub fn render_full_body(&self, buffer: &Buffer, writer: &mut dyn render::ChunkWriter) {
        let mut hunks = hunkify(self, None, buffer).peekable();
        let mut is_first = true;
        while let Some(hunk) = hunks.next() {
            let header = !is_first || hunks.peek().is_some();
            hunk.render(header, writer);
            is_first = false;
        }
    }

    pub fn is_unchanged(&self) -> bool {
        self.status_markers.len() == 1
    }

    fn status_idx_by_line(&self, new: bool, line: u32) -> usize {
        self.status_markers.partition_point(|sm| {
            let ref_line = if new { sm.new_line } else { sm.old_line };
            ref_line <= line
        }) - 1
    }

    /// Check whether the given `range` of lines in either the `old` or `new`
    /// file including a "halo" are matched completely to the other side of the
    /// match.
    ///
    /// The halo is defined as one line before and one line after the given range,
    /// possibly including a "line -1" before the start of the file.
    pub fn lines_unchanged_halo(&self, new: bool, range: Range<u32>) -> bool {
        let sm_idx = self.status_idx_by_line(new, range.start.saturating_sub(1));
        if range.start == 0 && sm_idx != 0 {
            return false;
        }

        let sm = &self.status_markers[sm_idx];
        let sm_next = self.status_markers.get(sm_idx + 1);
        !sm.is_changed() && sm_next.is_none_or(|sm_next| {
            let ref_line = if new { sm_next.new_line } else { sm_next.old_line };
            range.end < ref_line
        })
    }

    pub fn simplify(&mut self) {
        assert!(self.status_markers.last().unwrap().status == MatchStatus::Unchanged);
        for status_marker in std::mem::take(&mut self.status_markers) {
            if let Some(sm) = self.status_markers.last() {
                assert!(status_marker.old_line >= sm.old_line);
                assert!(status_marker.new_line >= sm.new_line);
                if sm.old_line == status_marker.old_line && sm.new_line == status_marker.new_line {
                    self.status_markers.pop();
                }
            } else {
                assert!(status_marker.old_line == 0 && status_marker.new_line == 0);
            }
            if self.status_markers.last().is_none_or(|sm| sm.status != status_marker.status) {
                self.status_markers.push(status_marker);
            }
        }
    }
}
