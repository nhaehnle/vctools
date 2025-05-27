use itertools::Itertools;
use ratatui::{prelude::*, text::Line, widgets::Block};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
};
use vctuik::{
    event::{self, Event, KeyCode, KeyEventKind, MouseEventKind},
    prelude::*,
    state::{Builder, Handled, Renderable},
    theme,
};

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

pub struct Pager<'source> {
    source: &'source dyn PagerSource,
}
impl<'source> Pager<'source> {
    pub fn new(source: &'source impl PagerSource) -> Self {
        Pager { source }
    }

    pub fn build<'render, 'handler, 'id, Id>(
        self,
        builder: &mut Builder<'_, 'render, 'handler>,
        id: Id,
        num_lines: u16,
    ) where
        Id: Into<Cow<'id, str>>,
        'source: 'render,
        'source: 'handler,
    {
        self.build_impl(builder, id.into(), num_lines);
    }

    fn build_impl<'render, 'handler>(
        self,
        builder: &mut Builder<'_, 'render, 'handler>,
        id: Cow<'_, str>,
        num_lines: u16,
    ) where
        'source: 'render,
        'source: 'handler,
    {
        let (id, state) = builder.add_state_widget::<PagerState, _>(id, true);
        let area = builder.take_lines(num_lines);

        state.last_height = area.height;

        let has_focus = builder.has_focus(id);

        builder.add_render(Renderable::Block(
            area,
            Block::default().style(builder.theme().pane_background),
        ));

        let num_lines = self.source.num_lines();
        let (scroll_row, scroll_col) = self.scroll(state, 0, 0);

        for ry in 0..area.height {
            let line_no = scroll_row + ry as usize;
            if line_no >= num_lines {
                break;
            }

            let line = self
                .source
                .get_line(builder.theme().text(builder.context()), line_no, scroll_col, area.width as usize)
                .style(builder.theme().text.normal);
            builder.add_render(Renderable::Line(
                Rect {
                    y: area.y + ry,
                    height: 1,
                    ..area
                },
                line,
            ));
        }

        let vertical_page_size = std::cmp::max((area.height / 2) as isize + 1,
                                               area.height as isize - 5);
        let horizontal_page_size = std::cmp::max(1, (area.width / 2) as isize);
        let mouse_page_size = std::cmp::min(5, vertical_page_size);

        builder.add_event_handler(move |event| {
            match event {
                Event::Key(ev) if has_focus && ev.kind == KeyEventKind::Press => {
                    match ev.code {
                        KeyCode::Up => {
                            self.scroll(state, -1, 0);
                        }
                        KeyCode::Down => {
                            self.scroll(state, 1, 0);
                        }
                        KeyCode::Left => {
                            self.scroll(state, 0, -horizontal_page_size);
                        }
                        KeyCode::Right => {
                            self.scroll(state, 0, horizontal_page_size);
                        }
                        KeyCode::PageUp => {
                            self.scroll(state, -vertical_page_size, 0);
                        }
                        KeyCode::PageDown => {
                            self.scroll(state, vertical_page_size, 0);
                        }
                        KeyCode::Home | KeyCode::Char('g') => {
                            state.scroll = Some(self.source.persist_cursor(0, 0, Gravity::Left));
                        }
                        KeyCode::End | KeyCode::Char('G') => {
                            let line = self.source.num_lines().saturating_sub(area.height as usize);
                            state.scroll = Some(self.source.persist_cursor(line, 0, Gravity::Left));
                        }
                        _ => return Handled::No,
                    }
                }

                Event::Mouse(ev) if area.contains(Position::new(ev.column, ev.row)) => {
                    match ev.kind {
                        MouseEventKind::ScrollDown => {
                            self.scroll(state, mouse_page_size, 0);
                        }
                        MouseEventKind::ScrollUp => {
                            self.scroll(state, -mouse_page_size, 0);
                        }
                        MouseEventKind::ScrollLeft => {
                            self.scroll(state, 0, -mouse_page_size);
                        }
                        MouseEventKind::ScrollRight => {
                            self.scroll(state, 0, mouse_page_size);
                        }
                        _ => return Handled::No,
                    }
                }

                _ => return Handled::No,
            }

            Handled::Yes
        });
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

    let running = Cell::new(true);

    while running.get() {
        terminal.run_frame(|builder| {
            Pager::new(&source).build(builder, "pager", builder.viewport().height);
            event::on_key_press(builder, KeyCode::Char('q'), |_| {
                running.set(false);
            });
        })?;
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
