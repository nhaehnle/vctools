// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fmt::Write, ops::Range};

use diff_modulo_base::{diff, git_core};
use ratatui::text::{Line, Span};
use vctuik::{
    pager::{self, PagerSource},
    prelude::*,
    theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffDisplayMode {
    All,
    OnlyOld,
    OnlyNew,
}
impl Default for DiffDisplayMode {
    fn default() -> Self {
        DiffDisplayMode::All
    }
}
impl DiffDisplayMode {
    fn toggled(self) -> Self {
        match self {
            DiffDisplayMode::All => DiffDisplayMode::OnlyOld,
            DiffDisplayMode::OnlyOld => DiffDisplayMode::OnlyNew,
            DiffDisplayMode::OnlyNew => DiffDisplayMode::All,
        }
    }

    fn is_covered(&self, status: diff::HunkLineStatus) -> bool {
        match self {
            DiffDisplayMode::All => true,
            DiffDisplayMode::OnlyOld => status.covers_old(),
            DiffDisplayMode::OnlyNew => status.covers_new(),
        }
    }

    fn show_old(&self) -> bool {
        matches!(self, DiffDisplayMode::All | DiffDisplayMode::OnlyOld)
    }

    fn show_new(&self) -> bool {
        matches!(self, DiffDisplayMode::All | DiffDisplayMode::OnlyNew)
    }
}

#[derive(Debug)]
enum Element {
    Chunk(diff::render::Chunk),
    Commit(git_core::RangeDiffMatch),
}
impl Element {
    fn num_lines(&self, mode: DiffDisplayMode) -> usize {
        match self {
            Element::Chunk(chunk) => match &chunk.contents {
                diff::render::ChunkContents::FileHeader { .. } => 2,
                diff::render::ChunkContents::HunkHeader { .. } => 1,
                diff::render::ChunkContents::Line { line } =>
                    if mode.is_covered(line.status) {
                        if line.contents.last().is_none_or(|ch| *ch != b'\n') {
                            2
                        } else {
                            1
                        }
                    } else {
                        0
                    },
            },
            Element::Commit(_) => 1,
        }
    }
}

#[derive(Default)]
pub struct DiffPagerSource {
    /// Flat list of all elements of the diff
    elements: Vec<Element>,

    /// Global (uncollapsed) line number for every element in `elements`,
    /// filtered according to `mode`.
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
}
impl std::fmt::Debug for DiffPagerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReviewPagerSource").finish_non_exhaustive()
    }
}
impl DiffPagerSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn num_global_lines(&self) -> usize {
        self.global_lines
            .last()
            .map_or(0, |&l| l + self.elements.last().unwrap().num_lines(self.mode))
    }

    pub fn toggle_mode(&mut self) {
        self.mode = self.mode.toggled();

        let mut line = 0;
        for (global_line, element) in self.global_lines.iter_mut().zip(self.elements.iter()) {
            *global_line = line;
            line += element.num_lines(self.mode);
        }
    }

    /// Find the nearest folding header at or below the given depth.
    ///
    /// If forward is true, find the smallest index strictly greater than the given index.
    ///
    /// If forward is false, find the largest index less than or equal to the given index.
    /// Returns (header_idx, depth).
    fn find_folding_header(
        &self,
        idx: usize,
        forward: bool,
        max_depth: usize,
    ) -> Option<(usize, usize)> {
        [&self.commits, &self.files, &self.hunks]
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
                if forward {
                    o.reverse()
                } else {
                    o
                }
            })
    }
}
impl diff::render::ChunkWriter for DiffPagerSource {
    fn push_chunk(&mut self, chunk: diff::render::Chunk) {
        self.global_lines.push(self.num_global_lines());

        if matches!(chunk.contents, diff::render::ChunkContents::FileHeader { .. }) {
            self.files.push(self.elements.len());
        } else if matches!(chunk.contents, diff::render::ChunkContents::HunkHeader { .. }) {
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

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line<'_> {
        let idx = self.global_lines.partition_point(|&l| l <= line) - 1;
        let line = line - self.global_lines[idx];

        let (text, style) = match &self.elements[idx] {
            Element::Chunk(chunk) =>
                match &chunk.contents {
                    diff::render::ChunkContents::HunkHeader { old_begin, old_count, new_begin, new_count } => {
                        let mut text = String::from_utf8_lossy(chunk.context.prefix_bytes()).to_string();
                        write!(&mut text, "@@").unwrap();
                        if self.mode.show_old() {
                            write!(&mut text, " -{},{}", old_begin, old_count).unwrap();
                        }
                        if self.mode.show_new() {
                            write!(&mut text, " +{},{}", new_begin, new_count).unwrap();
                        }
                        write!(&mut text, " @@").unwrap();
                        (text, theme.header2)
                    },
                    _ => {
                        let style = match &chunk.contents {
                            diff::render::ChunkContents::FileHeader { .. } => theme.header1,
                            diff::render::ChunkContents::Line { line } => match line.status {
                                diff::HunkLineStatus::Unchanged => theme.normal,
                                diff::HunkLineStatus::Old(_) => theme.removed,
                                diff::HunkLineStatus::New(_) => theme.added,
                            },
                            _ => unreachable!(),
                        };

                        let mut text = Vec::new();
                        chunk.render_text(&mut text);
                        (String::from_utf8_lossy(&text).into(), style)
                    }
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
            if idx == 0 || depth == 0 {
                return None;
            }
            (header_idx, depth) = self.find_folding_header(idx, false, depth - 1)?;
        }

        let end_idx = self.find_folding_header(header_idx, true, depth).unwrap().0;
        let end_line = if end_idx < self.global_lines.len() {
            self.global_lines[end_idx]
        } else {
            self.num_global_lines()
        };

        Some((self.global_lines[header_idx]..end_line, depth))
    }

    fn persist_line_number(&self, line: usize) -> (Vec<pager::Anchor>, usize) {
        let idx = self.global_lines.partition_point(|&l| l <= line) - 1;
        (vec![pager::Anchor::USize(idx)], line - self.global_lines[idx])
    }

    fn retrieve_line_number(&self, anchor: &[pager::Anchor], line_offset: usize) -> (usize, bool) {
        if anchor.len() != 1 {
            return (0, false);
        }
        let pager::Anchor::USize(idx) = anchor[0] else { return (0, false); };
        let Some(line) = self.global_lines.get(idx) else { return (self.num_global_lines(), false); };
        (*line + line_offset, true)
    }
}
