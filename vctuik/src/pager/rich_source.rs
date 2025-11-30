// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Debug;

use super::*;

use ratatui::text::Line;

enum Child<'a> {
    Borrowed(&'a dyn PagerSource),
    Owned(Box<dyn PagerSource + 'a>),
}
impl<'a> Debug for Child<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Child::Borrowed(_) => f.debug_tuple("Borrowed").finish_non_exhaustive(),
            Child::Owned(_) => f.debug_tuple("Owned").finish_non_exhaustive(),
        }
    }
}
impl<'a> std::ops::Deref for Child<'a> {
    type Target = dyn PagerSource + 'a;

    fn deref(&self) -> &Self::Target {
        match self {
            Child::Borrowed(value) => return *value,
            Child::Owned(value) => return value.as_ref(),
        }
    }
}

#[derive(Debug, Default)]
pub struct RichPagerSource<'text> {
    children: Vec<Child<'text>>,

    /// End line number (exclusive) of each child.
    end_lines: Vec<usize>,
}
impl<'text> RichPagerSource<'text> {
    pub fn new() -> Self {
        RichPagerSource::default()
    }

    fn add_child_impl(&mut self, child: Child<'text>) {
        let end_line = self.num_lines() + child.num_lines();
        self.children.push(child);
        self.end_lines.push(end_line);
    }

    pub fn add_child<S>(&mut self, child: S)
    where
        S: PagerSource + 'text,
    {
        self.add_child_impl(Child::Owned(Box::new(child)));
    }

    pub fn add_child_ref<S>(&mut self, child: &'text S)
    where
        S: PagerSource + 'text,
    {
        self.add_child_impl(Child::Borrowed(child));
    }

    fn get_child_idx_by_line(&self, line: usize) -> (usize, usize) {
        let idx = self.end_lines.partition_point(|l| *l <= line);
        let base_line = if idx == 0 {
            0
        } else {
            self.end_lines[idx - 1]
        };
        (base_line, idx)
    }

    fn get_child_by_line(&self, line: usize) -> (usize, Option<&dyn PagerSource>) {
        let (base_line, idx) = self.get_child_idx_by_line(line);
        (base_line, self.children.get(idx).map(|c| &**c))
    }
}
impl<'text> PagerSource for RichPagerSource<'text> {
    fn num_lines(&self) -> usize {
        self.end_lines.last().copied().unwrap_or(0)
    }

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line {
        let (base_line, child) = self.get_child_by_line(line);
        child.map(|child| {
            child.get_line(theme, line - base_line, col_no, max_cols)
        })
        .unwrap_or(Line::default())
    }

    fn get_raw_line(&self, line: usize, col_no: usize, max_cols: usize) -> Cow<'_, str> {
        let (base_line, child) = self.get_child_by_line(line);
        child.map(|child| {
            child.get_raw_line(line - base_line, col_no, max_cols)
        })
        .unwrap_or(Cow::Owned(String::new()))
    }

    fn get_folding_range(&self, line: usize, parent: bool) -> Option<(Range<usize>, usize)> {
        let (base_line, child) = self.get_child_by_line(line);
        child.and_then(|child| {
            child
                .get_folding_range(line - base_line, parent)
                .map(|(range, depth)| (range.start + base_line..range.end + base_line, depth))
        })
        .or(None)
    }

    fn persist_line_number(&self, line: usize) -> (Vec<Anchor>, usize) {
        let (base_line, idx) = self.get_child_idx_by_line(line);
        let (mut anchor, line_offset) =
            self.children.get(idx)
                .map(|child| child.persist_line_number(line - base_line))
                .unwrap_or((vec![], 0));
        anchor.push(Anchor::USize(idx));
        (anchor, line_offset)
    }

    fn retrieve_line_number(&self, anchor: &[Anchor], line_offset: usize) -> (usize, bool) {
        let Some((Anchor::USize(idx), anchor)) = anchor.split_last() else {
            return (0, false);
        };
        let Some(child) = self.children.get(*idx) else {
            return (0, false);
        };
        let (line, success) = child.retrieve_line_number(anchor, line_offset);
        let base_line = if *idx == 0 {
            0
        } else {
            self.end_lines[*idx - 1]
        };
        (line + base_line, success)
    }
}
