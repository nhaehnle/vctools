// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{
    event::{KeyCode, KeyModifiers, MouseButton, WithModifiers},
    layout::{Constraint1D, LayoutItem1D},
    prelude::*,
    state::{Builder, StateId},
    theme,
};
use itertools::Itertools;
use ratatui::{prelude::*, text::Line, widgets::Block};
use regex::Regex;
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::ops::Range;

use crate::command;
use crate::stringtools::StrScan;

/// A persistent cursor into a `PagerSource`.
///
/// This is used to remember a position in the pager source across frames even for pager sources
/// whose contents may change.
#[derive(Debug)]
pub struct PersistentCursor {
    id: usize,
}

/// Gravity of a persistent cursor.
///
/// Whether a cursor is anchored to the character to the left or to the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gravity {
    Left,
    Right,
}

/// Data backing for persistent cursors.
///
/// Stores a value of type `T` for each persistent cursor.
///
/// Implementations of `PagerSource` must use an instance of this type to manage persistent cursors.
#[derive(Debug, Default)]
pub struct PersistentCursors<T> {
    cursors: Vec<Option<T>>,
}
impl<T> PersistentCursors<T> {
    pub fn new() -> Self {
        PersistentCursors {
            cursors: Vec::new(),
        }
    }

    /// Update all persistent cursor backing data.
    ///
    /// Implementations may use this to update cursors when the underlying data changes if there
    /// is no other way to recover cursors' locations.
    pub fn update<F>(&mut self, f: F)
    where
        F: Fn(&mut T),
    {
        self.cursors.iter_mut().for_each(|cursor| {
            if let Some(ref mut c) = cursor {
                f(c);
            }
        });
    }

    /// Add a new persistent cursor and register its data.
    pub fn add(&mut self, data: T) -> PersistentCursor {
        for id in 0..self.cursors.len() {
            if self.cursors[id].is_none() {
                // Reuse an existing slot.
                self.cursors[id] = Some(data);
                return PersistentCursor { id };
            }
        }

        // No free slot found, allocate a new one.
        self.cursors.push(Some(data));
        PersistentCursor {
            id: self.cursors.len() - 1,
        }
    }

    /// Take and unregister the data for a persistent cursor.
    pub fn take(&mut self, cursor: PersistentCursor) -> T {
        // This may panic if a cursor is used with the incorrect `PersistentCursors` instance.
        //
        // If there is no confusion of `PersistentCursors` instances, this never panics because
        // cursors cannot be cloned or otherwise created outside of this module.
        self.cursors[cursor.id].take().unwrap()
    }
}

/// A source of data for the `Pager` widget.
///
/// `StringPagerSource` is a simple implementation sufficient for showing plain text.
///
/// Line and column numbers are 0-based.
pub trait PagerSource {
    /// Returns the number of lines in the pager source.
    fn num_lines(&self) -> usize;

    /// Returns a renderable presentation of the given line starting at the given column.
    ///
    /// As a hint, `max_cols` indicates a maximum number of characters that the caller is interested
    /// in. Implementations are encouraged not to return more data, even if the line is longer.
    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line;

    /// Returns the given line starting at the given column.
    ///
    /// As a hint, `max_cols` indicates a maximum number of characters that the caller is interested
    /// in. Implementations are encouraged not to return more data, even if the line is longer.
    fn get_raw_line(&self, line: usize, col_no: usize, max_cols: usize) -> Cow<'_, str> {
        let line = self.get_line(&theme::Text::unstyled(), line, col_no, max_cols);
        Cow::Owned(line.spans.iter().map(|span| span.content.as_ref()).join(""))
    }

    /// Get the folding range for the given line, if any.
    ///
    /// If `parent` is false, the inner-most folding range for the given line is returned.
    ///
    /// If `parent` is true and the given line is the *start* of a folding range, then
    /// the immediate parent folding range is returned instead (or None if there is no parent).
    ///
    /// Returns (range, depth) if a range was found.
    fn get_folding_range(&self, _line: usize, _parent: bool) -> Option<(Range<usize>, usize)> {
        None
    }

    /// Return a persistent cursor for the given line and column.
    ///
    /// This must accept column numbers beyond the end of the line. Such cursors should be treated
    /// as if the line had extra whitespace at the end.
    fn persist_cursor(&self, line: usize, col: usize, gravity: Gravity) -> PersistentCursor;

    /// Retrieve the current position of a persistent cursor.
    ///
    /// Returns ((line, column), anchor-removed), where `anchor-removed` indicates whether the
    /// character that the cursor was anchored to (based on gravity) was removed.
    fn retrieve_cursor(&self, cursor: PersistentCursor) -> ((usize, usize), bool);
}

#[derive(Debug, Default)]
pub struct PagerState {
    /// The top left corner of the view maps to this scroll position.
    ///
    /// Note that the scroll position may be hidden by sticky headers.
    scroll: Option<PersistentCursor>,
    select: Option<PersistentCursor>,
    last_height: u16,

    collapse: Vec<PersistentCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PagerColumn {
    Text(usize),
    Folding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PagerPoint {
    line: usize,
    sticky_header: bool,
    col: PagerColumn,
}

pub struct PagerResult<'result> {
    source: &'result dyn PagerSource,
    state: &'result mut PagerState,
    scroll: (usize, usize),
    select: usize,
    collapse: Vec<Range<usize>>,
    hidden: Vec<Range<usize>>,
}
impl<'result> Drop for PagerResult<'result> {
    fn drop(&mut self) {
        // Save the scroll position.
        self.state.scroll = Some(self.source.persist_cursor(
            self.scroll.0,
            self.scroll.1,
            Gravity::Left,
        ));
        self.state.select = Some(self.source.persist_cursor(self.select, 0, Gravity::Left));

        // Save the collapse state.
        self.state.collapse = self
            .collapse
            .iter()
            .map(|range| self.source.persist_cursor(range.start, 0, Gravity::Left))
            .collect();
    }
}
impl<'result> PagerResult<'result> {
    fn new(source: &'result dyn PagerSource, state: &'result mut PagerState) -> Self {
        let mut collapse = Vec::new();

        for cursor in std::mem::take(&mut state.collapse) {
            let (pos, removed) = source.retrieve_cursor(cursor);
            if removed {
                continue;
            }

            let Some((range, _)) = source.get_folding_range(pos.0, false) else {
                continue;
            };

            if range.start != pos.0 {
                continue;
            }

            collapse.push(range);
        }

        collapse.sort_by_key(|range| range.start);

        let scroll = state.scroll.take().map_or((0, 0), |cursor| {
            let (mut pos, removed) = source.retrieve_cursor(cursor);
            if removed {
                // Reset horizontal scroll if the containing line was removed.
                pos.1 = 0;
            }
            pos
        });

        let select = state
            .select
            .take()
            .map_or((0, 0), |cursor| source.retrieve_cursor(cursor).0);

        let mut result = PagerResult {
            source,
            state,
            scroll,
            select: select.0,
            collapse,
            hidden: Vec::new(),
        };

        result.update_hidden();
        result.normalize_scroll();

        let select = result.screen_forward(result.select).next().unwrap_or(0);
        result.select = select;

        result
    }

    fn update_hidden(&mut self) {
        self.hidden.clear();

        for range in &self.collapse {
            if range.len() <= 1 {
                continue;
            }

            let range = range.start + 1..range.end;

            if let Some(last) = self.hidden.last_mut() {
                if range.start < last.end {
                    assert!(range.end <= last.end);
                    continue;
                }
                if range.start == last.end {
                    last.end = range.end;
                    continue;
                }
            }
            self.hidden.push(range);
        }
    }

    fn is_collapsed(&self, line: usize) -> bool {
        let idx = self.collapse.partition_point(|range| range.start < line);
        if let Some(range) = self.collapse.get(idx) {
            return range.start == line;
        }
        return false;
    }

    fn set_collapsed(&mut self, line: usize, collapse: bool) {
        let idx = self.collapse.partition_point(|range| range.start < line);
        let old_collapsed = self
            .collapse
            .get(idx)
            .is_some_and(|range| range.start == line);
        if collapse == old_collapsed {
            return;
        }

        if collapse {
            let range = self.source.get_folding_range(line, false).unwrap().0;
            self.collapse.insert(idx, range.clone());

            // TODO: Do this more efficiently.
            self.update_hidden();
            self.normalize_scroll();

            let select = self
                .screen_forward(self.select)
                .next()
                .unwrap_or(self.select);
            self.select = select;
        } else {
            self.collapse.remove(idx);

            // TODO: Do this more efficiently. (Need to take parent ranges into account!)
            self.update_hidden();
        }
    }

    fn toggle_collapsed(&mut self, line: usize) {
        self.set_collapsed(line, !self.is_collapsed(line));
    }

    /// Starting with the given document line, iterate forward over lines as they
    /// would appear on the screen.
    ///
    /// The first line produced is the given line. If the given line is hidden,
    /// the nearest earlier visible line is produced first instead, or if there
    /// is no such line, the nearest later visible line.
    fn screen_forward<'a>(&'a self, mut line: usize) -> impl Iterator<Item = usize> + 'a {
        struct Forward<'fwd> {
            num_lines: usize,
            hidden: &'fwd [Range<usize>],
            hidden_idx: usize,
            line: usize,
        }
        impl<'fwd> Iterator for Forward<'fwd> {
            type Item = usize;

            fn next(&mut self) -> Option<Self::Item> {
                let num_lines = self.num_lines;
                if self.line >= num_lines {
                    return None;
                }

                let result = self.line;

                self.line += 1;
                if self.hidden_idx < self.hidden.len() {
                    assert!(self.line < self.hidden[self.hidden_idx].end);
                    if self.hidden[self.hidden_idx].start <= self.line {
                        self.line = self.hidden[self.hidden_idx].end;
                        self.hidden_idx += 1;

                        assert!(
                            self.hidden_idx >= self.hidden.len()
                                || self.line < self.hidden[self.hidden_idx].start
                        );
                    }
                }

                Some(result)
            }
        }

        let hidden_idx = self.hidden.partition_point(|range| range.end <= line);
        if let Some(range) = self.hidden.get(hidden_idx) {
            if 0 < range.start && range.start <= line {
                line = range.start - 1;
            }
        }

        Forward {
            num_lines: self.source.num_lines(),
            hidden: &self.hidden,
            hidden_idx,
            line,
        }
    }

    /// Iterate backward from the given document line over lines as they would appear
    /// on the screen.
    ///
    /// If the given document line is hidden, it is replaced by the nearest earlier
    /// visible line.
    ///
    /// The given line (or its replacement) are not produced by the iterator. If the
    /// given line is the first visible line, the iterator is empty.
    ///
    /// screen_forward and screen_backward starting from the same document line
    /// together produce all visible lines in the document exactly once.
    fn screen_backward<'a>(&'a self, mut line: usize) -> impl Iterator<Item = usize> + 'a {
        struct Backward<'fwd> {
            hidden: &'fwd [Range<usize>],
            hidden_idx: usize,
            line: usize,
        }
        impl<'fwd> Iterator for Backward<'fwd> {
            type Item = usize;

            fn next(&mut self) -> Option<Self::Item> {
                if self.line == 0 {
                    return None;
                }

                self.line -= 1;

                if self.hidden_idx > 0 {
                    assert!(self.hidden[self.hidden_idx - 1].start <= self.line);
                    if self.line < self.hidden[self.hidden_idx - 1].end {
                        self.line = self.hidden[self.hidden_idx - 1].start;
                        self.hidden_idx -= 1;

                        if self.line == 0 {
                            return None;
                        }
                        self.line -= 1;

                        assert!(
                            self.hidden_idx == 0
                                || self.hidden[self.hidden_idx - 1].end <= self.line
                        );
                    }
                }

                Some(self.line)
            }
        }

        let hidden_idx = self.hidden.partition_point(|range| range.end <= line);
        if let Some(range) = self.hidden.get(hidden_idx) {
            if range.start <= line {
                line = range.start.saturating_sub(1);
            }
        }

        Backward {
            hidden: &self.hidden,
            hidden_idx,
            line,
        }
    }

    /// Computes the sticky headers as (document line, depth) pairs.
    fn compute_sticky_headers(&self) -> Vec<(usize, usize)> {
        let mut sticky_headers = Vec::new();

        let mut screen = self.screen_forward(self.scroll.0).peekable();
        let Some(&first_line) = screen.peek() else {
            return sticky_headers;
        };

        // Add headers for ranges that contain the first screen line.
        if let Some((range, depth)) = self.source.get_folding_range(first_line, true) {
            sticky_headers.push((range.start, depth));

            let mut line = range.start;
            loop {
                let Some((range, depth)) = (line > 0)
                    .then(|| self.source.get_folding_range(line, true))
                    .flatten()
                else {
                    break;
                };

                sticky_headers.push((range.start, depth));
                line = range.start;
            }
        }

        sticky_headers.reverse();

        // Iterate over lines on the screen that are covered by sticky headers
        // and adjust the headers accordingly.
        for (y, line) in screen.enumerate() {
            if y >= sticky_headers.len() {
                // This line is not covered by sticky headers, so we can break.
                break;
            }

            if let Some((range, depth)) = self.source.get_folding_range(line, false) {
                if range.start == line {
                    // This is the header of a folding range, and it is covered
                    // by sticky headers.
                    while let Some(&(_, d)) = sticky_headers.last() {
                        if d >= depth {
                            sticky_headers.pop();
                        } else {
                            break;
                        }
                    }
                    if y >= sticky_headers.len() {
                        break;
                    }
                    sticky_headers.push((range.start, depth));
                }
            }
        }

        let max_headers = (self.state.last_height as usize).saturating_sub(10);
        sticky_headers.drain(0..sticky_headers.len().saturating_sub(max_headers));

        sticky_headers
    }

    /// Current selection as a range of document lines.
    fn selection(&self) -> Range<usize> {
        let num_lines = self.source.num_lines();
        let start = self.select;
        let end = std::cmp::min(start + 1, num_lines);
        start..end
    }

    /// Classify the (x, y) mouse position relative to the pager area.
    fn classify_point(&self, x: u16, y: u16) -> PagerPoint {
        let x = x as usize;
        let y = y as usize;

        let sticky_headers = self.compute_sticky_headers();
        if y < sticky_headers.len() {
            return PagerPoint {
                line: sticky_headers[y].0,
                sticky_header: true,
                col: PagerColumn::Text(x),
            };
        }

        let line = self
            .screen_forward(self.scroll.0)
            .skip(y)
            .next()
            .unwrap_or_else(|| self.source.num_lines().saturating_sub(1));

        let col = if x == 0 {
            PagerColumn::Folding
        } else {
            PagerColumn::Text(self.scroll.1 + x - 1)
        };
        PagerPoint {
            line,
            sticky_header: false,
            col,
        }
    }

    /// Normalize the scroll position to ensure:
    /// - it is 0 or a visible line
    /// - we don't scroll past the end of the document
    fn normalize_scroll(&mut self) {
        if self.state.last_height == 0 {
            return;
        }

        let mut last_y = 0;
        for (y, _) in self
            .screen_forward(self.scroll.0)
            .enumerate()
            .take(self.state.last_height as usize)
        {
            last_y = y + 1;
        }
        assert!(last_y <= self.state.last_height as usize);

        let rewind = self.state.last_height as usize - last_y;
        if rewind != 0 {
            let line = self
                .screen_backward(self.scroll.0)
                .skip(rewind - 1)
                .next()
                .unwrap_or(0);
            self.scroll.0 = line;
        }
    }

    /// Scroll by the given number of screen lines and columns.
    fn scroll_by(&mut self, lines: isize, cols: isize) -> (usize, usize) {
        self.scroll.1 = self.scroll.1.saturating_add_signed(cols);

        if lines < 0 {
            let line = self
                .screen_backward(self.scroll.0)
                .skip((-lines - 1) as usize)
                .next()
                .unwrap_or(0);
            self.scroll.0 = line;
        } else if lines > 0 {
            let line = self
                .screen_forward(self.scroll.0)
                .skip(lines as usize)
                .next()
                .unwrap_or_else(|| self.source.num_lines().saturating_sub(1));
            self.scroll.0 = line;
            self.normalize_scroll();
        }

        self.scroll
    }

    /// Scroll the given document line into view.
    ///
    /// Scrolling into view means scrolling such that the line is in the desired
    /// range on screen if possible (i.e., not too close to the top or bottom, and
    /// not hidden by sticky headers).
    ///
    /// If the given document line is hidden, it is replaced by the nearest earlier visible line.
    fn scroll_line_into_view(&mut self, line: usize) {
        // Normalize to a visible line.
        let Some(line) = self.screen_forward(line).next() else {
            return;
        };

        let total_height = self.state.last_height as usize;

        // Determine whether we're outside of the desired range.
        let o = if line < self.scroll.0 {
            Ordering::Less
        } else {
            let y = self
                .screen_forward(self.scroll.0)
                .enumerate()
                .take(total_height)
                .skip_while(|&(_, l)| l < line)
                .next()
                .map(|(y, _)| y)
                .unwrap_or(total_height);

            if y >= total_height {
                Ordering::Greater
            } else {
                let sticky_headers = self.compute_sticky_headers();
                let body_height = total_height.saturating_sub(sticky_headers.len());
                let margin = std::cmp::min(4, body_height / 3);

                if y < sticky_headers.len() + margin {
                    Ordering::Less
                } else if y >= total_height.saturating_sub(margin) {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            }
        };

        // Scroll up or down as required.
        match o {
            Ordering::Less => {
                let margin = std::cmp::min(4, total_height / 3);
                let Some(new_scroll) = std::iter::once(line)
                    .chain(self.screen_backward(line))
                    .skip(margin)
                    .next()
                else {
                    self.scroll.0 = 0;
                    return;
                };
                self.scroll.0 = new_scroll;

                let mut y = margin;

                loop {
                    let sticky_headers = self.compute_sticky_headers();
                    let body_height = total_height.saturating_sub(sticky_headers.len());
                    let margin = std::cmp::min(4, body_height / 3);

                    if y >= sticky_headers.len() + margin {
                        break;
                    }

                    let Some(new_scroll) = self.screen_backward(self.scroll.0).next() else {
                        self.scroll.0 = 0;
                        break;
                    };
                    self.scroll.0 = new_scroll;
                    y += 1;
                }
            }
            Ordering::Greater => {
                let margin = std::cmp::min(4, total_height / 3);
                let margin = self.screen_forward(line).skip(1).take(margin).count();
                let target_y = total_height.saturating_sub(margin + 1);
                let new_scroll = std::iter::once(line)
                    .chain(self.screen_backward(line))
                    .skip(target_y)
                    .next()
                    .unwrap_or(0);
                self.scroll.0 = new_scroll;
            }
            Ordering::Equal => {
                // nothing to do
            }
        }
    }

    /// Move the selection by the given number of screen lines.
    fn move_by(&mut self, lines: isize) {
        let select = self.screen_forward(self.select).next().unwrap_or(0);

        let select = if lines < 0 {
            self.screen_backward(select)
                .skip((-lines - 1) as usize)
                .next()
                .unwrap_or_else(|| self.screen_forward(0).next().unwrap_or(0))
        } else if lines > 0 {
            self.screen_forward(select)
                .skip(lines as usize)
                .next()
                .unwrap_or_else(|| {
                    let last_line = self.source.num_lines().saturating_sub(1);
                    self.screen_forward(last_line).next().unwrap_or(last_line)
                })
        } else {
            select
        };

        self.select = select;

        self.scroll_line_into_view(self.select);
    }

    fn move_to_no_scroll(&mut self, line: usize) -> usize {
        // Normalize to a visible line, or 0 if nothing is visible.
        let line = self.screen_forward(line).next().unwrap_or(0);
        self.select = line;
        self.select
    }

    pub fn move_to(&mut self, line: usize) {
        let line = self.move_to_no_scroll(line);
        self.scroll_line_into_view(line);
    }

    fn search_impl(&self, pattern: &Regex, line: usize) -> bool {
        let line = self.source.get_raw_line(line, 0, usize::MAX);
        pattern.find(&line).is_some()
    }

    pub fn search(&mut self, pattern: &Regex, forward: bool) {
        let line = if forward {
            (self.select + 1..self.source.num_lines()).find(|&line| self.search_impl(pattern, line))
        } else {
            (0..self.select)
                .rev()
                .find(|&line| self.search_impl(pattern, line))
        };
        if let Some(line) = line {
            self.move_to(line);
        }
    }
}

pub struct Pager<'build, 'result> {
    source: &'result dyn PagerSource,
    search_pattern: Option<Cow<'build, Regex>>,
}
impl<'build, 'result> Pager<'build, 'result> {
    pub fn new(source: &'result impl PagerSource) -> Self {
        Pager {
            source,
            search_pattern: None,
        }
    }

    pub fn search(self, pattern: impl Into<Cow<'build, Regex>>) -> Self {
        Self {
            search_pattern: Some(pattern.into()),
            ..self
        }
    }

    pub fn build<'id>(self, builder: &mut Builder, id: impl Into<Cow<'id, str>>) {
        let state_id = builder.add_state_id(id);
        let state: &mut PagerState = builder.get_state(state_id);
        self.build_impl(builder, state_id, state);
    }

    pub fn build_with_state<'builder, 'state, 'id>(
        self,
        builder: &'builder mut Builder,
        id: impl Into<Cow<'id, str>>,
        state: &'state mut PagerState,
    ) -> PagerResult<'result>
    where
        'state: 'result,
    {
        let state_id = builder.add_state_id(id);
        self.build_impl(builder, state_id, state)
    }

    pub fn build_impl<'builder, 'state>(
        self,
        builder: &'builder mut Builder,
        state_id: StateId,
        state: &'state mut PagerState,
    ) -> PagerResult<'result>
    where
        'state: 'result,
    {
        let area =
            builder.take_lines(LayoutItem1D::new(Constraint1D::new_min(5)).id(state_id, true));
        let has_focus = builder.check_focus(state_id);

        state.last_height = area.height;

        let mut result = PagerResult::new(self.source, state);

        // Handle events
        let vertical_page_size =
            std::cmp::max((area.height / 2) as isize + 1, area.height as isize - 5);
        let horizontal_page_size = std::cmp::max(1, (area.width / 2) as isize);
        let mouse_page_size = std::cmp::min(5, vertical_page_size);

        if has_focus {
            if builder.on_key_press(KeyCode::Left) {
                if result.scroll.1 > 0 {
                    result.scroll_by(0, -horizontal_page_size);
                } else if let Some((range, _)) = self.source.get_folding_range(result.select, false)
                {
                    if result.select != range.start {
                        result.move_to(range.start);
                    } else {
                        if !result.is_collapsed(result.select) {
                            result.set_collapsed(result.select, true);
                        } else if let Some((range, _)) =
                            self.source.get_folding_range(result.select, true)
                        {
                            result.move_to(range.start);
                        }
                    }
                }
            }
            if builder.on_key_press(KeyCode::Right) {
                if result.is_collapsed(result.select) {
                    result.set_collapsed(result.select, false);
                } else {
                    result.scroll_by(0, horizontal_page_size);
                }
            }
            if builder.on_key_press(KeyCode::Up.with_modifiers(KeyModifiers::ALT)) {
                result.scroll_by(-1, 0);
            }
            if builder.on_key_press(KeyCode::Down.with_modifiers(KeyModifiers::ALT)) {
                result.scroll_by(1, 0);
            }
            if builder.on_key_press(KeyCode::PageUp.with_modifiers(KeyModifiers::ALT)) {
                result.scroll_by(-vertical_page_size, 0);
            }
            if builder.on_key_press(KeyCode::PageDown.with_modifiers(KeyModifiers::ALT)) {
                result.scroll_by(vertical_page_size, 0);
            }
            if builder.on_key_press(KeyCode::Up) {
                result.move_by(-1);
            }
            if builder.on_key_press(KeyCode::Down) {
                result.move_by(1);
            }
            if builder.on_key_press(KeyCode::PageUp) {
                result.move_by(-vertical_page_size);
            }
            if builder.on_key_press(KeyCode::PageDown) {
                result.move_by(vertical_page_size);
            }
            if builder.on_key_press_any(&[KeyCode::Home.into(), KeyCode::Char('g').into()]) {
                result.move_to(0);
            }
            if builder.on_key_press_any(&[KeyCode::End.into(), KeyCode::Char('G').into()]) {
                let line = self.source.num_lines().saturating_sub(1);
                result.move_to(line);
            }
            if builder.on_key_press(KeyCode::Char('n')) {
                if let Some(pattern) = &self.search_pattern {
                    result.search(pattern, true);
                }
            }
            if builder.on_key_press(KeyCode::Char('N')) {
                if let Some(pattern) = &self.search_pattern {
                    result.search(pattern, false);
                }
            }
        }

        if builder.on_mouse_scroll_down(area).is_some() {
            result.scroll_by(mouse_page_size, 0);
        }
        if builder.on_mouse_scroll_up(area).is_some() {
            result.scroll_by(-mouse_page_size, 0);
        }
        if builder.on_mouse_scroll_left(area).is_some() {
            result.scroll_by(0, -mouse_page_size);
        }
        if builder.on_mouse_scroll_right(area).is_some() {
            result.scroll_by(0, mouse_page_size);
        }

        if let Some(pos) = builder.on_mouse_press(area, MouseButton::Left) {
            let point = result.classify_point(pos.x - area.x, pos.y - area.y);

            match point.col {
                PagerColumn::Text(_) => {
                    let line = result.move_to_no_scroll(point.line);
                    if point.sticky_header {
                        result.scroll_line_into_view(line);
                    }
                }
                PagerColumn::Folding => {
                    if self
                        .source
                        .get_folding_range(point.line, false)
                        .is_some_and(|(r, _)| r.start == point.line)
                    {
                        result.toggle_collapsed(point.line);
                    }
                }
            }

            builder.grab_focus(state_id);
        }

        // Render widget
        let block = Block::default().style(builder.theme().pane_background);
        builder.frame().render_widget(
            block,
            Rect {
                x: area.x.saturating_add(1),
                width: area.width.saturating_sub(1),
                ..area
            },
        );

        let block = Block::default().style(builder.theme().modal_background);
        builder
            .frame()
            .render_widget(block, Rect { width: 1, ..area });

        let num_lines = self.source.num_lines();
        let selection = result.selection();

        let sticky_headers = result.compute_sticky_headers();

        for (ry, &(line_no, _)) in sticky_headers.iter().enumerate() {
            let y = area.y + ry as u16;

            let block = Block::default().style(builder.theme().modal_background);
            builder.frame().render_widget(
                block,
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );

            let line = self.source.get_line(
                builder.theme().text(builder.theme_context()),
                line_no,
                result.scroll.1,
                area.width as usize,
            );
            builder.frame().render_widget(
                line,
                Rect {
                    y,
                    height: 1,
                    ..area
                },
            );
        }

        let text_width = area.width.saturating_sub(1);

        for (ry, line_no) in result
            .screen_forward(result.scroll.0)
            .enumerate()
            .take(result.state.last_height as usize)
            .skip(sticky_headers.len())
        {
            let y = area.y + (ry as u16);
            assert!(line_no < num_lines);

            if selection.contains(&line_no) {
                let block =
                    Block::default().style(builder.theme().text(builder.theme_context()).selected);
                builder.frame().render_widget(
                    block,
                    Rect {
                        y,
                        height: 1,
                        ..area
                    },
                );
            }

            if let Some((range, _)) = self.source.get_folding_range(line_no, false) {
                if range.start == line_no {
                    // Render the folding range marker.
                    let marker = match result.is_collapsed(line_no) {
                        true => "▶",
                        false => "▼",
                    };
                    let span = Span::from(marker).style(builder.theme().modal_text.normal);
                    builder.frame().render_widget(
                        span,
                        Rect {
                            y,
                            height: 1,
                            ..area
                        },
                    );
                }
            }

            if let Some(pattern) = &self.search_pattern {
                // Get the complete line and extract the raw text.
                //
                // TODO: This loses a whole bunch of the efficiency that PagerSource was built for.
                let line = self.source.get_line(
                    builder.theme().text(builder.theme_context()),
                    line_no,
                    0,
                    usize::MAX,
                );
                let text = line
                    .spans
                    .iter()
                    .map(|span: &Span| span.content.as_ref())
                    .join("");

                // Find matches.
                let matches = pattern
                    .find_iter(&text)
                    .map(|m| m.range())
                    .collect::<Vec<_>>();

                // Combine the pre-existing styles with matches and render spans on the fly.
                let search = builder.theme().text(builder.theme_context()).search;
                let normal = builder.theme().text(builder.theme_context()).normal;

                let mut styles = line
                    .spans
                    .iter()
                    .scan(0, |idx, span| {
                        let start = *idx;
                        *idx += span.content.len();
                        Some((start, span.style))
                    })
                    .peekable();
                let mut matches = matches
                    .iter()
                    .flat_map(|m| [(m.start, true), (m.end, false)])
                    .peekable();

                let mut base_style = Style::default();
                let mut in_match = false;
                let mut row_col = text.row_col_scan((0, 0)).peekable();

                'line: while let Some(&((_, mut begin_col), mut begin_idx)) = row_col.peek() {
                    let (next_style_idx, next_style) = styles
                        .peek()
                        .map(Clone::clone)
                        .unwrap_or((usize::MAX, Style::default()));
                    let (next_match_idx, next_match) = matches
                        .peek()
                        .map(Clone::clone)
                        .unwrap_or((usize::MAX, false));
                    let end_idx =
                        std::cmp::min(text.len(), std::cmp::min(next_style_idx, next_match_idx));

                    while begin_col < result.scroll.1 && begin_idx < end_idx {
                        row_col.next();
                        if let Some(&((_, col), idx)) = row_col.peek() {
                            begin_col = col;
                            begin_idx = idx;
                        } else {
                            break 'line;
                        }
                    }

                    if begin_col >= result.scroll.1 + text_width as usize {
                        break;
                    }

                    if begin_idx != end_idx {
                        let style = if in_match {
                            if base_style == normal {
                                search
                            } else {
                                search.patch(base_style)
                            }
                        } else {
                            base_style
                        };

                        let span = Span::from(&text[begin_idx..end_idx]).style(style);
                        let rx = (begin_col - result.scroll.1) as u16;
                        builder.frame().render_widget(
                            span,
                            Rect {
                                y,
                                x: area.x + 1 + rx,
                                width: text_width - 1 - rx,
                                height: 1,
                            },
                        );

                        while begin_idx < end_idx {
                            row_col.next();
                            #[allow(unused_assignments)]
                            if let Some(&((_, col), idx)) = row_col.peek() {
                                begin_col = col;
                                begin_idx = idx;
                            } else {
                                break 'line;
                            }
                        }
                    }

                    if end_idx == next_style_idx {
                        base_style = normal.patch(next_style);
                        styles.next();
                    } else if end_idx == next_match_idx {
                        in_match = next_match;
                        matches.next();
                    }
                }
            } else {
                let line = self
                    .source
                    .get_line(
                        builder.theme().text(builder.theme_context()),
                        line_no,
                        result.scroll.1,
                        text_width as usize,
                    )
                    .style(builder.theme().text.normal);

                builder.frame().render_widget(
                    line,
                    Rect {
                        y,
                        height: 1,
                        x: area.x + 1,
                        width: text_width,
                    },
                );
            }
        }

        result
    }
}

pub struct StringPagerSource<'text> {
    text: &'text str,

    /// ((line number, column number), byte offset into text)
    /// Last entry is at end of text
    anchors: Vec<((usize, usize), usize)>,

    cursors: RefCell<PersistentCursors<(usize, usize)>>,
}
impl<'text> StringPagerSource<'text> {
    pub fn new(text: &'text str) -> Self {
        let mut pos = (0, 0);
        let mut anchors: Vec<_> = text
            .row_col_scan_mut(&mut pos)
            .chunks(512)
            .into_iter()
            .map(|chunk| chunk.into_iter().next().unwrap())
            .collect();
        anchors.push((pos, text.len()));

        StringPagerSource {
            text,
            anchors,
            cursors: RefCell::new(PersistentCursors::new()),
        }
    }

    /// Return the byte offset of the first character past the given line and column.
    fn get_index(&self, line: usize, col: usize) -> usize {
        let anchor = self
            .anchors
            .partition_point(|&((l, c), _)| l < line || (l == line && c <= col));
        if anchor == 0 {
            assert!(self.text.is_empty());
            return 0;
        }

        let (pos, anchor_offset) = self.anchors[anchor - 1];
        match self.text[anchor_offset..]
            .row_col_scan(pos)
            .find(|&(pos, _)| pos.0 > line || (pos.0 == line && pos.1 >= col))
        {
            Some(((found_line, found_col), offset)) => {
                let byte_offset = anchor_offset + offset;
                if found_line != line {
                    // The target column is past the end of the line, so we rewind past the newline.
                    assert!(found_line == line + 1 && found_col == 0);
                    byte_offset - 1
                } else {
                    byte_offset
                }
            }
            None => {
                // If we didn't find a character at the given position, return the end of the text.
                self.text.len()
            }
        }
    }
}
impl<'text> PagerSource for StringPagerSource<'text> {
    fn num_lines(&self) -> usize {
        let eof = self.anchors.last().unwrap().0;
        if eof.1 == 0 {
            eof.0
        } else {
            eof.0 + 1
        }
    }

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line {
        let start = self.get_index(line, col_no);
        Line::from(self.text[start..].get_first_line(max_cols)).style(theme.normal)
    }

    fn persist_cursor(&self, line: usize, col: usize, _gravity: Gravity) -> PersistentCursor {
        self.cursors.borrow_mut().add((line, col))
    }

    fn retrieve_cursor(&self, cursor: PersistentCursor) -> ((usize, usize), bool) {
        let pos = self.cursors.borrow_mut().take(cursor);
        (pos, false)
    }
}

pub fn run(text: String) -> Result<()> {
    let mut terminal = crate::init()?;
    let source = StringPagerSource::new(&text);

    let mut running = true;
    let mut pager_state = PagerState::default();
    let mut search: Option<Regex> = None;
    let mut error: Option<String> = None;
    let mut command: Option<String> = None;

    terminal.run(|builder| {
        let mut pager = Pager::new(&source);
        if let Some(regex) = &search {
            pager = pager.search(Cow::Borrowed(regex));
        }
        let mut pager_result = pager.build_with_state(builder, "pager", &mut pager_state);

        let was_search = command.as_ref().is_some_and(|cmd| cmd.starts_with('/'));

        let action = command::CommandLine::new("command", &mut command)
            .help("/ to search, q to quit")
            .build(builder, |builder, _| {
                if let Some(error) = &error {
                    let area = builder.take_lines_fixed(1);
                    let span = Span::from(error)
                        .style(builder.theme().text(builder.theme_context()).error);
                    builder.frame().render_widget(span, area);
                }
            });
        match action {
            command::CommandAction::None => {}
            command::CommandAction::Command(_) => {
                if was_search {
                    if let Some(pattern) = &search {
                        pager_result.search(pattern, true);
                    }
                }
                error = None;
            }
            command::CommandAction::Changed(cmd) => {
                assert!(!cmd.is_empty());

                error = None;
                if cmd.starts_with('/') {
                    search = None;
                    if cmd.len() > 1 {
                        match Regex::new(&cmd[1..]) {
                            Ok(regex) => {
                                search = Some(regex);
                            }
                            Err(e) => {
                                error = Some(format!("{}", e));
                            }
                        }
                    }
                } else {
                    error = Some(format!(
                        "Unknown command prefix: {}",
                        cmd.chars().next().unwrap()
                    ));
                }
                builder.need_refresh();
            }
            command::CommandAction::Cancelled => {
                if was_search {
                    search = None;
                }
                error = None;
            }
        }

        // Global key bindings
        if builder.on_key_press(KeyCode::Char('/')) {
            command = Some("/".into());
            search = None;
            builder.need_refresh();
        }
        if builder.on_key_press(KeyCode::Char('q')) {
            running = false;
        }

        Ok(running)
    })?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn sps_basic() {
        let filler: String = std::iter::repeat('+')
            .take(500)
            .chain(std::iter::once('\n'))
            .collect();
        let text = "First line\n".to_owned() + &filler + "Third line\n" + &filler + "Fifth line\n";
        let source = StringPagerSource::new(&text);
        let theme = theme::Theme::default().text;
        assert_eq!(source.num_lines(), 5);
        assert_eq!(
            source.get_line(&theme, 0, 0, usize::MAX).to_string(),
            "First line"
        );
        assert_eq!(
            source.get_line(&theme, 2, 3, usize::MAX).to_string(),
            "rd line"
        );
        assert_eq!(
            source.get_line(&theme, 4, 3, usize::MAX).to_string(),
            "th line"
        );
        assert_eq!(source.get_line(&theme, 0, 10, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 0, 11, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 2, 10, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 2, 11, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 4, 10, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 5, 0, usize::MAX).width(), 0);
        assert!(source.get_line(&theme, 0, 0, 3).width() >= 3);
    }

    #[test]
    fn sps_empty() {
        let source = StringPagerSource::new("");
        let theme = theme::Theme::default().text;
        assert_eq!(source.num_lines(), 0);
        assert_eq!(source.get_line(&theme, 0, 0, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 0, 0, 3).width(), 0);
        assert_eq!(source.get_line(&theme, 1, 0, usize::MAX).width(), 0);
        assert_eq!(source.get_line(&theme, 1, 0, 3).width(), 0);
    }
}
