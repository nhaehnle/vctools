// SPDX-License-Identifier: GPL-3.0-or-later

use std::ops::Range;

use diff_modulo_base::{diff, git_core};
use ratatui::text::{Line, Span};
use vctuik::{
    pager::{self, PagerSource}, prelude::*, theme
};

#[derive(Debug)]
enum Element {
    ReviewHeader(String),
    Chunk(diff::Chunk),
    Commit(git_core::RangeDiffMatch),
}
impl Element {
    fn num_lines(&self) -> usize {
        match self {
            Element::ReviewHeader(text) => text.lines().count(),
            Element::Chunk(chunk) => match &chunk.contents {
                diff::ChunkContents::FileHeader { .. } => 2,
                _ => 1,
            },
            Element::Commit(_) => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffDisplayMode {
    Unified,
    OnlyOld,
    OnlyNew,
}
impl Default for DiffDisplayMode {
    fn default() -> Self {
        DiffDisplayMode::Unified
    }
}
impl DiffDisplayMode {
    fn toggled(self) -> Self {
        match self {
            DiffDisplayMode::Unified => DiffDisplayMode::OnlyOld,
            DiffDisplayMode::OnlyOld => DiffDisplayMode::OnlyNew,
            DiffDisplayMode::OnlyNew => DiffDisplayMode::Unified,
        }
    }
}

#[derive(Default)]
pub struct DiffPagerSource {
    /// Flat list of all elements of the diff
    elements: Vec<Element>,

    /// Global (uncollapsed) line number for every element in `elements`
    global_lines: Vec<usize>,

    /// Indices into `elements` of commit headers
    commits: Vec<usize>,

    /// Indices into `elements` of all file headers
    files: Vec<usize>,

    /// Indices into `elements` of all hunk headers
    hunks: Vec<usize>,

    mode: DiffDisplayMode,

    /// Column widths for range diff matches
    rdm_column_widths: git_core::RangeDiffMatchColumnWidths,

    /// Persistent cursors
    cursors: std::cell::RefCell<pager::PersistentCursors<(usize, usize, bool)>>,
}
impl std::fmt::Debug for DiffPagerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewPagerSource")
            .finish_non_exhaustive()
    }
}
impl DiffPagerSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn num_global_lines(&self) -> usize {
        self.global_lines
            .last()
            .map_or(0, |&l| l + self.elements.last().unwrap().num_lines())
    }

    pub fn push_header(&mut self, text: String) {
        assert!(self.elements.is_empty());
        self.global_lines.push(self.num_global_lines());
        self.elements.push(Element::ReviewHeader(text));
    }

    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.toggled();
        todo!()
    }

    pub fn truncate_to_header(&mut self) {
        self.elements.truncate(1);
        self.global_lines.truncate(1);
        self.commits.clear();
        self.files.clear();
        self.hunks.clear();
        self.rdm_column_widths = git_core::RangeDiffMatchColumnWidths::default();

        let num_lines = self.num_global_lines();
        let end =
            if num_lines > 0 {
                (num_lines - 1, self.get_raw_line(num_lines - 1, 0, usize::MAX).as_ref().graphemes(true).count())
            } else {
                (0, 0)
            };

        self.cursors.borrow_mut().update(|cursor| {
            if cursor.0 >= num_lines {
                *cursor = (end.0, end.1, true);
            }
        });
    }

    /// Find the nearest folding header at or below the given depth.
    ///
    /// If forward is true, find the smallest index strictly greater than the given index.
    /// 
    /// If forward is false, find the largest index less than or equal to the given index.
    /// Returns (header_idx, depth).
    fn find_folding_header(&self, idx: usize, forward: bool, max_depth: usize) -> Option<(usize, usize)> {
        [
            &self.commits,
            &self.files,
            &self.hunks,
        ]
        .into_iter()
        .take(max_depth.saturating_add(1))
        .enumerate()
        .filter_map(|(depth, indices)| {
            let i = indices.partition_point(|&i| i <= idx);
            if forward {
                if i < indices.len() {
                    Some((indices[i], depth))
                } else {
                    Some((self.elements.len(), 0))
                }
            } else {
                if i == 0 {
                    None
                } else {
                    Some((indices[i - 1], depth))
                }
            }
        })
        .max_by(|a, b| {
            let o = a.0.cmp(&b.0);
            if forward { o.reverse() } else { o }
        })
    }
}
impl diff::ChunkWriter for DiffPagerSource {
    fn push_chunk(&mut self, chunk: diff::Chunk) {
        self.global_lines.push(self.num_global_lines());

        if matches!(chunk.contents, diff::ChunkContents::FileHeader { .. }) {
            self.files.push(self.elements.len());
        } else if matches!(chunk.contents, diff::ChunkContents::HunkHeader { .. }) {
            self.hunks.push(self.elements.len());
        }

        self.elements.push(Element::Chunk(chunk));
    }
}
impl git_core::RangeDiffWriter for DiffPagerSource {
    fn push_range_diff_match(&mut self, rdm: git_core::RangeDiffMatch) {
        self.rdm_column_widths = self.rdm_column_widths.max(rdm.column_widths());

        self.global_lines.push(self.num_global_lines());
        self.commits.push(self.elements.len());
        self.elements.push(Element::Commit(rdm));
    }
}
impl PagerSource for DiffPagerSource {
    fn num_lines(&self) -> usize {
        self.num_global_lines()
    }

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line {
        let idx = self.global_lines.partition_point(|&l| l <= line) - 1;
        let line = line - self.global_lines[idx];

        let (text, style) = match &self.elements[idx] {
            Element::ReviewHeader(text) => (text.clone(), theme.highlight),
            Element::Chunk(chunk) => {
                let style = match &chunk.contents {
                    diff::ChunkContents::FileHeader { .. } => theme.header1,
                    diff::ChunkContents::HunkHeader { .. } => theme.header2,
                    diff::ChunkContents::Line { line } => match line.status {
                        diff::HunkLineStatus::Unchanged => theme.normal,
                        diff::HunkLineStatus::Old(_) => theme.removed,
                        diff::HunkLineStatus::New(_) => theme.added,
                    },
                };

                let mut text = Vec::new();
                chunk.render_text(&mut text);

                (String::from_utf8_lossy(&text).into(), style)
            }
            Element::Commit(rdm) => (rdm.format(self.rdm_column_widths), theme.header0),
        };

        let offset = text
            .row_col_scan((0, 0))
            .find_map(|((l, c), offset)| {
                if l > line {
                    Some(None)
                } else if l == line && c >= col_no {
                    Some(Some(offset))
                } else {
                    None
                }
            })
            .unwrap_or(None);
        let Some(offset) = offset else {
            return Line::default();
        };

        let text = text[offset..].get_first_line(max_cols);
        Line::from(Span::styled(text.to_owned(), style))
    }

    fn get_folding_range(&self, line: usize, parent: bool) -> Option<(Range<usize>, usize)> {
        let idx = self.global_lines.partition_point(|&l| l <= line) - 1;
        let line = line - self.global_lines[idx];

        let (mut header_idx, mut depth) = self.find_folding_header(idx, false, usize::MAX)?;
        if parent && header_idx == idx && line == 0 {
            if idx == 0 || depth == 0{
                return None;
            }
            (header_idx, depth) = self.find_folding_header(idx, false, depth - 1)?;
        }

        let end_idx = self.find_folding_header(header_idx, true, depth).unwrap().0;
        let end_line =
            if end_idx < self.global_lines.len() {
                self.global_lines[end_idx]
            } else {
                self.num_global_lines()
            };

        Some((self.global_lines[header_idx]..end_line, depth))
    }

    fn persist_cursor(
        &self,
        line: usize,
        col: usize,
        _gravity: pager::Gravity,
    ) -> pager::PersistentCursor {
        self.cursors.borrow_mut().add((line, col, false))
    }

    fn retrieve_cursor(&self, cursor: pager::PersistentCursor) -> ((usize, usize), bool) {
        let (line, col, removed) = self.cursors.borrow_mut().take(cursor);
        ((line, col), removed)
    }
}
