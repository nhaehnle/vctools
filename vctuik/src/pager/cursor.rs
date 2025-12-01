// SPDX-License-Identifier: GPL-3.0-or-later

use std::any::Any;

use super::PagerSource;

#[derive(Debug)]
pub enum Anchor {
    String(String),
    Str(&'static str),
    USize(usize),
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
    pub fn persist<S>(source: &S, line: usize, col: usize) -> Self
    where
        S: PagerSource + ?Sized,
    {
        let (anchor, line_offset) = source.persist_line_number(line);
        PersistentCursor {
            anchor,
            line_offset,
            col,
        }
    }

    pub fn retrieve<S>(&self, source: &S) -> ((usize, usize), bool)
    where
        S: PagerSource + ?Sized,
    {
        let (mut line, mut success) = source.retrieve_line_number(&self.anchor, self.line_offset);

        let max_line = source.num_lines().saturating_sub(1);
        if line > max_line {
            line = max_line;
            success = false;
        }

        let raw_line = source.get_raw_line(line, 0, self.col);
        let max_col = raw_line.len().saturating_sub(1);

        let mut col = self.col;
        if col > max_col {
            col = max_col;
            success = false;
        }
        
        ((line, col), success)
    }
}
