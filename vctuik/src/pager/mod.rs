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
use std::cmp::Ordering;
use std::ops::Range;

use crate::command;
use crate::stringtools::StrScan;

mod string_source;
mod widget;

pub use string_source::StringPagerSource;
pub use widget::{Pager, PagerState};

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
