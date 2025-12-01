// SPDX-License-Identifier: GPL-3.0-or-later

mod rich_source;
mod cursor;
mod string_source;
mod widget;

pub use rich_source::{RichPagerSource, RichPagerSourceBuilder};
pub use cursor::{Anchor, Cursor, PersistentCursor};
pub use string_source::StringPagerSource;
pub use widget::{Pager, PagerState};

use std::borrow::Cow;
use std::ops::Range;
use itertools::Itertools;
use ratatui::prelude::*;
use regex::Regex;

use crate::{command, event::KeyCode, prelude::*, theme};
use crate::stringtools::StrScan;

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

    /// Return (anchor, line_offset) for the given line.
    ///
    /// This is used to persist cursors across frames. The intention is that the
    /// anchor is reasonably stable even when the contents of the pager source change.
    fn persist_line_number(&self, line: usize) -> (Vec<Anchor>, usize);

    /// Retrieve the line number for the given anchor and line offset.
    ///
    /// Return (line, success), where `success` is false if the anchor (or parts of it)
    /// was removed.
    fn retrieve_line_number(&self, anchor: &[Anchor], line_offset: usize) -> (usize, bool);
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
