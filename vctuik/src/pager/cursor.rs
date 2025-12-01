// SPDX-License-Identifier: GPL-3.0-or-later

use std::any::Any;

use super::PagerSource;

#[derive(Debug)]
pub enum Anchor {
    String(String),
    Str(&'static str),
    USize(usize),
    USize2(usize, usize),
    Any(Box<dyn Any + 'static>),
}
impl Anchor {
    pub fn from_any<T: 'static>(value: T) -> Self {
        Anchor::Any(Box::new(value))
    }

    pub fn as_any<T: 'static>(&self) -> Option<&T> {
        match self {
            Anchor::Any(boxed) => boxed.downcast_ref::<T>(),
            _ => None,
        }
    }

    pub fn into_any<T: 'static>(self) -> Option<T> {
        match self {
            Anchor::Any(boxed) => boxed.downcast::<T>().ok().map(|b| *b),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
}
impl Cursor {
    pub fn new(line: usize, col: usize) -> Self {
        Cursor { line, col }
    }
}
impl std::cmp::PartialOrd for Cursor {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl std::cmp::Ord for Cursor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.line.cmp(&other.line) {
            std::cmp::Ordering::Equal => self.col.cmp(&other.col),
            ord => ord,
        }
    }
}

/// A persistent cursor into a `PagerSource`.
///
/// This is used to remember a position in the pager source across frames even for pager sources
/// whose contents may change.
#[derive(Debug)]
pub struct PersistentCursor {
    anchor: Vec<Anchor>,
    line_offset: usize,
    col: usize,
}
impl PersistentCursor {
    pub fn persist<S>(source: &S, pos: Cursor) -> Self
    where
        S: PagerSource + ?Sized,
    {
        let (anchor, line_offset) = source.persist_line_number(pos.line);
        PersistentCursor {
            anchor,
            line_offset,
            col: pos.col,
        }
    }

    pub fn retrieve<S>(&self, source: &S) -> (Cursor, bool)
    where
        S: PagerSource + ?Sized,
    {
        let (mut line, mut success) = source.retrieve_line_number(&self.anchor, self.line_offset);

        let max_line = source.num_lines().saturating_sub(1);
        if line > max_line {
            line = max_line;
            success = false;
        }
        
        (Cursor::new(line, self.col), success)
    }
}
