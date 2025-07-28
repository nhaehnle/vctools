// SPDX-License-Identifier: GPL-3.0-or-later

use itertools::Itertools;
use ratatui::{prelude::*, text::Line, widgets::Block};
use regex::Regex;
use std::borrow::Cow;
use std::{
    cell::RefCell,
};
use vctuik::{
    event::KeyCode, layout::{Constraint1D, LayoutItem1D}, prelude::*, state::Builder, theme
};

use crate::stringtools::StrScan;
use crate::command;

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
struct PagerState {
    scroll: Option<PersistentCursor>,
    last_height: u16,
}

pub struct Pager<'pager> {
    source: &'pager dyn PagerSource,
    search_pattern: Option<Cow<'pager, Regex>>,
}
impl<'pager> Pager<'pager> {
    pub fn new(source: &'pager impl PagerSource) -> Self {
        Pager {
            source,
            search_pattern: None,
        }
    }

    pub fn search(self, pattern: impl Into<Cow<'pager, Regex>>) -> Self {
        Self {
            search_pattern: Some(pattern.into()),
            ..self
        }
    }

    pub fn build(self, builder: &mut Builder, id: impl Into<String>)
    {
        let state_id = builder.add_state_id(id.into().into());
        let state: &mut PagerState = builder.get_state(state_id);
        let area = builder.take_lines(LayoutItem1D::new(Constraint1D::new_min(5)).id(state_id, true));
        let has_focus = builder.check_focus(state_id);

        state.last_height = area.height;

        // Handle events
        let vertical_page_size = std::cmp::max((area.height / 2) as isize + 1,
                                               area.height as isize - 5);
        let horizontal_page_size = std::cmp::max(1, (area.width / 2) as isize);
        let mouse_page_size = std::cmp::min(5, vertical_page_size);

        if has_focus {
            if builder.on_key_press(KeyCode::Up) {
                self.scroll(state, -1, 0);
            }
            if builder.on_key_press(KeyCode::Down) {
                self.scroll(state, 1, 0);
            }
            if builder.on_key_press(KeyCode::Left) {
                self.scroll(state, 0, -horizontal_page_size);
            }
            if builder.on_key_press(KeyCode::Right) {
                self.scroll(state, 0, horizontal_page_size);
            }
            if builder.on_key_press(KeyCode::PageUp) {
                self.scroll(state, -vertical_page_size, 0);
            }
            if builder.on_key_press(KeyCode::PageDown) {
                self.scroll(state, vertical_page_size, 0);
            }
            if builder.on_key_press_any(&[KeyCode::Home, KeyCode::Char('g')]) {
                state.scroll = Some(self.source.persist_cursor(0, 0, Gravity::Left));
            }
            if builder.on_key_press_any(&[KeyCode::End, KeyCode::Char('G')]) {
                let line = self.source.num_lines().saturating_sub(area.height as usize);
                state.scroll = Some(self.source.persist_cursor(line, 0, Gravity::Left));
            }
        }

        if builder.on_mouse_scroll_down(area).is_some() {
            self.scroll(state, mouse_page_size, 0);
        }
        if builder.on_mouse_scroll_up(area).is_some() {
            self.scroll(state, -mouse_page_size, 0);
        }
        if builder.on_mouse_scroll_left(area).is_some() {
            self.scroll(state, 0, -mouse_page_size);
        }
        if builder.on_mouse_scroll_right(area).is_some() {
            self.scroll(state, 0, mouse_page_size);
        }

        // Render widget
        let block = Block::default().style(builder.theme().pane_background);
        builder.frame().render_widget(block, area);

        let num_lines = self.source.num_lines();
        let (scroll_row, scroll_col) = self.scroll(state, 0, 0);

        for ry in 0..area.height {
            let line_no = scroll_row + ry as usize;
            if line_no >= num_lines {
                break;
            }

            if let Some(pattern) = &self.search_pattern {
                // Get the complete line and extract the raw text.
                //
                // TODO: This loses a whole bunch of the efficiency that PagerSource was built for.
                let line =
                    self.source.get_line(
                        builder.theme().text(builder.context()),
                        line_no, 0, usize::MAX);
                let text = line.spans.iter()
                    .map(|span: &Span| span.content.as_ref())
                    .join("");

                // Find matches.
                let matches =
                    pattern.find_iter(&text)
                        .map(|m| m.range())
                        .collect::<Vec<_>>();

                // Combine the pre-existing styles with matches and render spans on the fly.
                let selected = builder.theme().text(builder.context()).selected;
                let normal = builder.theme().text(builder.context()).normal;

                let mut styles = line.spans.iter()
                    .scan(0, |idx, span| {
                        let start = *idx;
                        *idx += span.content.len();
                        Some((start, span.style))
                    })
                    .peekable();
                let mut matches = matches.iter()
                    .flat_map(|m| [(m.start, true), (m.end, false)])
                    .peekable();

                let mut base_style = Style::default();
                let mut in_match = false;
                let mut row_col = text.row_col_scan((0, 0)).peekable();

                'line: while let Some(&((_, mut begin_col), mut begin_idx)) = row_col.peek() {
                    let (next_style_idx, next_style) =
                        styles.peek()
                            .map(Clone::clone)
                            .unwrap_or((usize::MAX, Style::default()));
                    let (next_match_idx, next_match) =
                        matches.peek()
                            .map(Clone::clone)
                            .unwrap_or((usize::MAX, false));
                    let end_idx = std::cmp::min(text.len(), std::cmp::min(next_style_idx, next_match_idx));

                    while begin_col < scroll_col && begin_idx < end_idx {
                        row_col.next();
                        if let Some(&((_, col), idx)) = row_col.peek() {
                            begin_col = col;
                            begin_idx = idx;
                        } else {
                            break 'line;
                        }
                    }

                    if begin_col >= scroll_col + area.width as usize {
                        break;
                    }

                    if begin_idx != end_idx {
                        let style =
                            if in_match {
                                if base_style == normal {
                                    selected
                                } else {
                                    selected.patch(base_style)
                                }
                            } else {
                                base_style
                            };

                        let span = Span::from(&text[begin_idx..end_idx]).style(style);
                        let rx = (begin_col - scroll_col) as u16;
                        builder.frame().render_widget(
                            span,
                            Rect {
                                y: area.y + ry,
                                x: area.x + rx,
                                width: area.width - rx,
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
                let line =
                    self.source
                        .get_line(
                            builder.theme().text(builder.context()),
                            line_no,
                            scroll_col,
                            area.width as usize
                        )
                        .style(builder.theme().text.normal);

                builder.frame().render_widget(
                    line,
                    Rect {
                        y: area.y + ry,
                        height: 1,
                        ..area
                    },
                );
            }
        }
    }

    fn scroll(&self, state: &mut PagerState, lines: isize, cols: isize) -> (usize, usize) {
        let mut pos = state.scroll.take().map_or((0, 0), |cursor| {
            self.source.retrieve_cursor(cursor).0
        });

        pos.0 = pos.0.saturating_add_signed(lines);
        pos.1 = pos.1.saturating_add_signed(cols);

        if pos.0.saturating_add(state.last_height as usize) >= self.source.num_lines() {
            pos.0 = self.source.num_lines().saturating_sub(state.last_height as usize);
        }

        state.scroll = Some(self.source.persist_cursor(pos.0, pos.1, Gravity::Left));

        pos
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
    let mut terminal = vctuik::init()?;
    let source = StringPagerSource::new(&text);

    let mut running = true;
    let mut command: Option<String> = None;

    while running {
        while terminal.run_frame(|builder| {
            let mut pager = Pager::new(&source);
            if let Some(command) = &command {
                if command.starts_with('/') {
                    if command.len() > 1 {
                        if let Ok(regex) = Regex::new(&command[1..]) {
                            pager = pager.search(Cow::Owned(regex));
                        } else {
                            //builder.show_error(format!("Invalid search pattern: {}", command));
                        }
                    }
                } else {
                    //builder.show_error(format!("Unknown command: {}", command));
                }
            }
            pager.build(builder, "pager");

            match command::CommandLine::new("command", &mut command)
                .help("/ to search, q to quit")
                .build(builder) {
            command::CommandAction::None => {},
            command::CommandAction::Command(cmd) => {
                // TODO
            },
            command::CommandAction::Changed => {
                builder.need_refresh();
            },
            }

            // Global key bindings
            if builder.on_key_press(KeyCode::Char('/')) {
                command = Some("/".into());
                builder.need_refresh();
            }
            if builder.on_key_press(KeyCode::Char('q')) {
                running = false;
                return;
            }
        })? {
            // until state has settled
        }
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn sps_basic() {
        let filler: String = std::iter::repeat('+').take(500).chain(std::iter::once('\n')).collect();
        let text =
            "First line\n".to_owned() +
            &filler +
            "Third line\n" +
            &filler +
            "Fifth line\n";
        let source = StringPagerSource::new(&text);
        let theme = theme::Theme::default().text;
        assert_eq!(source.num_lines(), 5);
        assert_eq!(source.get_line(&theme, 0, 0, usize::MAX).to_string(), "First line");
        assert_eq!(source.get_line(&theme, 2, 3, usize::MAX).to_string(), "rd line");
        assert_eq!(source.get_line(&theme, 4, 3, usize::MAX).to_string(), "th line");
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
