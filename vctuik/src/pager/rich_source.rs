// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, cmp::Ordering, collections::HashMap, fmt::{Debug, Write}};

use super::*;

use ratatui::{symbols::line, text::Line};
use vctools_utils::prelude::*;

enum Element<'a> {
    PagerRef(&'a dyn PagerSource),
    Pager(Box<dyn PagerSource + 'a>),
    String(String),
    Str(&'a str),
}
impl<'a> Debug for Element<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Element::PagerRef(_) => f.debug_tuple("PagerRef").finish_non_exhaustive(),
            Element::Pager(_) => f.debug_tuple("Pager").finish_non_exhaustive(),
            Element::String(_) => f.debug_tuple("String").finish_non_exhaustive(),
            Element::Str(_) => f.debug_tuple("Str").finish_non_exhaustive(),
        }
    }
}
impl<'a> Element<'a> {
    fn as_ref<'b>(&'b self) -> ElementRef<'b> {
        match self {
            Element::PagerRef(value) => ElementRef::Pager(*value),
            Element::Pager(value) => ElementRef::Pager(value.as_ref()),
            Element::String(value) => ElementRef::Str(value.as_str()),
            Element::Str(value) => ElementRef::Str(*value),
        }
    }

    fn as_string_mut(&mut self) -> Option<&mut String> {
        match self {
            Element::String(s) => Some(s),
            _ => None,
        }
    }
}

enum ElementRef<'a> {
    Pager(&'a dyn PagerSource),
    Str(&'a str),
}
impl<'a> Debug for ElementRef<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElementRef::Pager(_) => f.debug_tuple("Pager").finish_non_exhaustive(),
            ElementRef::Str(_) => f.debug_tuple("Str").finish_non_exhaustive(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CustomStyleId(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Themed(theme::TextStyle),
    Custom(CustomStyleId),
}
impl Default for Style {
    fn default() -> Self {
        Style::Themed(theme::TextStyle::Normal)
    }
}

#[derive(Debug)]
pub struct RichPagerSourceBuilder<'text> {
    content: Vec<Element<'text>>,

    /// (content index, indent) pairs
    element_indent: Vec<(usize, usize)>,

    custom_styles: Vec<style::Style>,

    custom_style_map: HashMap<style::Style, CustomStyleId>,

    style: Vec<(Index, Style)>,
}
impl<'text> Default for RichPagerSourceBuilder<'text> {
    fn default() -> Self {
        RichPagerSourceBuilder {
            content: Vec::new(),
            element_indent: Vec::new(),
            custom_styles: Vec::new(),
            custom_style_map: HashMap::new(),
            style: vec![(
                Index { element: 0, offset: 0 },
                Style::default(),
            )],
        }
    }
}
impl<'text> RichPagerSourceBuilder<'text> {
    pub fn new() -> Self {
        Self::default()
    }

    fn add_impl(&mut self, element: Element<'text>) {
        self.content.push(element);
    }

    pub fn add_child<S>(&mut self, child: S)
    where
        S: PagerSource + 'text,
    {
        self.add_impl(Element::Pager(Box::new(child)));
    }

    pub fn add_child_ref<S>(&mut self, child: &'text S)
    where
        S: PagerSource + 'text,
    {
        self.add_impl(Element::PagerRef(child));
    }

    pub fn add_text(&mut self, text: &'text str) {
        self.add_impl(Element::Str(text));
    }

    pub fn register_style(&mut self, style: style::Style) -> Style {
        if let Some(id) = self.custom_style_map.get(&style) {
            return Style::Custom(*id);
        }

        let id = CustomStyleId(self.custom_styles.len() as u32);
        self.custom_styles.push(style);
        self.custom_style_map.insert(style, id);
        Style::Custom(id)
    }

    pub fn set_style(&mut self, style: Style) {
        let idx = if let Some(Element::String(s)) = self.content.last() {
            Index {
                element: self.content.len() - 1,
                offset: s.len(),
            }
        } else {
            Index {
                element: self.content.len(),
                offset: 0,
            }
        };
        if let Some(last) = self.style.last_mut() {
            if last.0 == idx {
                last.1 = style;
                return;
            }
        }
        self.style.push((idx, style));
    }

    pub fn set_theme_style(&mut self, style: theme::TextStyle) {
        self.set_style(Style::Themed(style))
    }

    pub fn clear_style(&mut self) {
        self.set_style(Style::default())
    }

    pub fn set_indent(&mut self, indent: usize) {
        if let Some(last) = self.element_indent.last_mut() {
            if last.0 == self.content.len() {
                last.1 = indent;
                return;
            }
        }
        self.element_indent.push((self.content.len(), indent));
    }

    pub fn build(mut self) -> RichPagerSource<'text> {
        // Compute landmarks. We introduce one landmark at the beginning of
        // each element. Additionally, each string element gets an anchor every
        // N bytes.
        let mut line_indent = Vec::new();
        let mut indent_iter = self.element_indent.into_iter().peekable();

        if let Some((_, indent)) = indent_iter.peek().filter(|(idx, _)| *idx == 0) {
            line_indent.push((0, *indent));
            indent_iter.next();
        } else {
            line_indent.push((0, 0));
        }

        let mut line = 0;
        let mut landmarks = Vec::new();
        for (idx, element) in self.content.iter().enumerate() {
            if let Some((_, indent)) = indent_iter.peek().filter(|(i, _)| *i == idx) {
                line_indent.push((line, *indent));
                indent_iter.next();
            }

            match element.as_ref() {
                ElementRef::Pager(pager) => {
                    landmarks.push(Landmark {
                        pos: Cursor {
                            line: line,
                            col: 0,
                        },
                        idx: Index {
                            element: idx,
                            offset: 0,
                        },
                    });
                    line += pager.num_lines();
                }
                ElementRef::Str(s) => {
                    let mut tup_pos = (line, 0);
                    for chunk in s.row_col_scan_mut(&mut tup_pos).chunks(512).into_iter() {
                        let first = chunk.into_iter().next().unwrap();
                        landmarks.push(Landmark {
                            pos: Cursor {
                                line: first.0.0,
                                col: first.0.1,
                            },
                            idx: Index {
                                element: idx,
                                offset: first.1,
                            },
                        });
                    }
                    if tup_pos.1 == 0 {
                        line = tup_pos.0;
                    } else {
                        line = tup_pos.0 + 1;
                    }
                }
            }
        }
        landmarks.push(Landmark {
            pos: Cursor {
                line,
                col: 0,
            },
            idx: Index {
                element: self.content.len(),
                offset: 0,
            },
        });

        self.style.push((
            Index {
                element: self.content.len(),
                offset: 0,
            },
            Style::default(),
        ));

        RichPagerSource {
            content: self.content,
            line_indent,
            landmarks,
            custom_styles: self.custom_styles,
            style: self.style,
            ..Default::default()
        }
    }
}
impl Write for RichPagerSourceBuilder<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let string = 'str: {
            if self.element_indent.last().is_none_or(|(idx, _)| *idx < self.content.len()) {
                if let Some(Element::String(string)) = self.content.last_mut() {
                    break 'str string;
                }
            }
            self.add_impl(Element::String(String::new()));
            self.content.last_mut().unwrap().as_string_mut().unwrap()
        };
        string.write_str(s)
    }
}

/// Reference an index in the rich source by element and offset within the element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Index {
    /// Index in the `elements` vector.
    element: usize,

    /// For string elements, the byte offset within the string.
    offset: usize,
}
impl std::cmp::PartialOrd for Index {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl std::cmp::Ord for Index {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.element.cmp(&other.element) {
            std::cmp::Ordering::Equal => self.offset.cmp(&other.offset),
            ord => ord,
        }
    }
}

/// Landmarks are used to quickly:
///  * find the element containing a given line, and the offset within the element
///  * given an element and offset, find the corresponding line number
#[derive(Debug, Clone, PartialEq, Eq)]
struct Landmark {
    pos: Cursor,
    idx: Index,
}

#[derive(Debug)]
pub struct RichPagerSource<'text> {
    content: Vec<Element<'text>>,
    landmarks: Vec<Landmark>,

    /// Most recent lookup result, used to prime the next lookup.
    ///
    /// Contains (index, cache point). `index` is the index of the largest landmark
    /// (in the landmarks vector) that is before or equal to the cache point.
    lm_lookup_cache: RefCell<(usize, Landmark)>,

    custom_styles: Vec<style::Style>,
    style: Vec<(Index, Style)>,
    style_lookup_cache: RefCell<usize>,

    /// Line number -> indent
    line_indent: Vec<(usize, usize)>,
    line_indent_lookup_cache: RefCell<usize>,
}
impl Default for RichPagerSource<'_> {
    fn default() -> Self {
        RichPagerSource {
            content: Vec::new(),
            landmarks: vec![
                Landmark {
                    pos: Cursor {
                        line: 0,
                        col: 0,
                    },
                    idx: Index {
                        element: 0,
                        offset: 0,
                    },
                },
            ],
            lm_lookup_cache: RefCell::new((0, Landmark {
                pos: Cursor {
                    line: 0,
                    col: 0,
                },
                idx: Index {
                    element: 0,
                    offset: 0,
                },
            })),

            custom_styles: Vec::new(),
            style: Vec::new(),
            style_lookup_cache: RefCell::new(0),

            line_indent: Vec::new(),
            line_indent_lookup_cache: RefCell::new(0),
        }
    }
}
impl RichPagerSource<'_> {
    pub fn new() -> Self {
        RichPagerSource::default()
    }

    /// Return the index into the `style` vector that determines the styole at the given index.
    fn style_idx_from_idx(&self, idx: Index) -> usize {
        let lookup_cache = &mut *self.style_lookup_cache.borrow_mut();
        let forward = self.style[*lookup_cache].0 <= idx;
        let style_idx = self.style.partition_point_with_hint(*lookup_cache, forward, |s| s.0 <= idx) - 1;
        *lookup_cache = style_idx;
        style_idx
    }

    fn line_indent(&self, line: usize) -> usize {
        let lookup_cache = &mut *self.line_indent_lookup_cache.borrow_mut();
        let forward = self.line_indent[*lookup_cache].0 <= line;
        let indent_idx = self.line_indent.partition_point_with_hint(*lookup_cache, forward, |&(l, _)| l <= line) - 1;
        *lookup_cache = indent_idx;
        self.line_indent[indent_idx].1
    }

    /// Return the (element index, offset) of the first character past the given line and
    /// (unindented!) column.
    fn idx_from_pos(&self, pos: Cursor) -> Index {
        let lookup_cache = &mut *self.lm_lookup_cache.borrow_mut();

        let (hint_idx, forward) = if lookup_cache.1.pos <= pos {
            (lookup_cache.0, true)
        } else {
            (lookup_cache.0 + 1, false)
        };
        let lm_next_idx = self.landmarks.partition_point_with_hint(hint_idx, forward, |lm| lm.pos <= pos);
        let lm_idx = lm_next_idx - 1;
        let lm = &self.landmarks[lm_idx];

        match self.content.get(lm.idx.element).map(Element::as_ref) {
            Some(ElementRef::Pager(_)) => {
                let offset = pos.line - lm.pos.line;

                lookup_cache.0 = lm_idx;
                lookup_cache.1 = lm.clone();

                Index {
                    element: lm.idx.element,
                    offset,
                }
            }
            Some(ElementRef::Str(s)) => {
                // Reuse the offset from the cache if possible.
                let lm =
                    if forward && lm_idx == lookup_cache.0 {
                        lookup_cache.1.clone()
                    } else {
                        lm.clone()
                    };

                match s[lm.idx.offset..]
                    .row_col_scan((lm.pos.line, lm.pos.col))
                    .map(|(p, o)| (Cursor { line: p.0, col: p.1 }, o))
                    .find(|&(p, _)| p >= pos)
                {
                    Some((found_pos, found_offset)) => {
                        let mut offset = lm.idx.offset + found_offset;

                        lookup_cache.0 = lm_idx;
                        lookup_cache.1 = Landmark {
                            pos: found_pos,
                            idx: Index {
                                element: lm.idx.element,
                                offset,
                            },
                        };

                        if found_pos.line != pos.line {
                            // The target column is past the end of the line, so we rewind past the newline.
                            debug_assert!(found_pos.line == pos.line + 1 && found_pos.col == 0);
                            offset -= 1
                        }

                        Index {
                            element: lm.idx.element,
                            offset,
                        }
                    }
                    None => {
                        // If we didn't find a character at the given position, return the end of the text.
                        Index {
                            element: lm.idx.element,
                            offset: s.len(),
                        }
                    }
                }
            }
            None => Index {
                element: self.content.len(),
                offset: 0,
            },
        }
    }

    /// Return the (unindented!) line and column for the given index.
    fn pos_from_idx(&self, idx: Index) -> Cursor {
        let lookup_cache = &mut *self.lm_lookup_cache.borrow_mut();

        let (hint_idx, forward) = if lookup_cache.1.idx <= idx {
            (lookup_cache.0, true)
        } else {
            (lookup_cache.0 + 1, false)
        };
        let lm_next_idx = self.landmarks.partition_point_with_hint(hint_idx, forward, |lm| lm.idx <= idx);
        let lm_idx = lm_next_idx - 1;
        let lm = &self.landmarks[lm_idx];
        assert!(lm.idx.element == idx.element);

        match self.content.get(idx.element).map(Element::as_ref) {
            Some(ElementRef::Pager(_)) => {
                assert!(lm.idx.offset == 0);

                lookup_cache.0 = lm_idx;
                lookup_cache.1 = lm.clone();

                Cursor {
                    line: lm.pos.line + idx.offset,
                    col: 0,
                }
            }
            Some(ElementRef::Str(s)) => {
                let lm =
                    if forward && lm_idx == lookup_cache.0 {
                        lookup_cache.1.clone()
                    } else {
                        lm.clone()
                    };

                // Find the last character with a byte offset less than or equal
                // to the target offset.
                let rel_offset = idx.offset - lm.idx.offset;
                let (found_pos, found_offset) = s[lm.idx.offset..]
                    .row_col_scan((lm.pos.line, lm.pos.col))
                    .map(|(p, o)| (Cursor { line: p.0, col: p.1 }, o))
                    .take_while(|&(_, o)| o <= rel_offset)
                    .fold((lm.pos, 0), |_, (p, o)| (p, o));

                lookup_cache.0 = lm_idx;
                lookup_cache.1 = Landmark {
                    pos: found_pos,
                    idx: Index {
                        element: idx.element,
                        offset: lm.idx.offset + found_offset,
                    },
                };

                found_pos
            }
            None => lm.pos,
        }
    }
}
impl<'text> PagerSource for RichPagerSource<'text> {
    fn num_lines(&self) -> usize {
        self.landmarks.last().unwrap().pos.line
    }

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line<'_> {
        let line_indent = self.line_indent(line);
        let indent = line_indent.saturating_sub(col_no);
        let col_no = col_no.saturating_sub(line_indent);

        let idx = self.idx_from_pos(Cursor { line, col: col_no });

        match self.content.get(idx.element).map(Element::as_ref) {
            Some(ElementRef::Pager(pager)) => {
                let mut line = pager.get_line(theme, idx.offset, col_no, max_cols);
                if indent != 0 {
                    line.spans.insert(0, Span::styled(" ".repeat(indent), theme.normal));
                }
                line
            }
            Some(ElementRef::Str(s)) => {
                let text = s[idx.offset..].get_first_line(max_cols);
                let style_idx = self.style_idx_from_idx(idx);
                let spans_iter = self.style[style_idx..].iter()
                    .zip(self.style[style_idx+1..].iter())
                    .map_while(|(style, next_style)| {
                        let start = match style.0.element.cmp(&idx.element) {
                            Ordering::Less => 0,
                            Ordering::Equal => style.0.offset,
                            Ordering::Greater => usize::MAX,
                        };
                        let end = match next_style.0.element.cmp(&idx.element) {
                            Ordering::Less => 0,
                            Ordering::Equal => next_style.0.offset,
                            Ordering::Greater => usize::MAX,
                        };
                        let start = start.saturating_sub(idx.offset);
                        let end = end.saturating_sub(idx.offset).min(text.len());
                        if start >= end {
                            return None;
                        }
                        let span_text = &text[start..end];
                        let span_style = match style.1 {
                            Style::Themed(ts) => theme[ts],
                            Style::Custom(id) => {
                                self.custom_styles[id.0 as usize]
                            }
                        };
                        Some(Span::styled(span_text, span_style))
                    });
                (indent != 0).then(|| Span::styled(" ".repeat(indent), theme.normal))
                    .into_iter()
                    .chain(spans_iter)
                    .collect()
            }
            None => Line::default(),
        }
    }

    fn get_raw_line(&self, line: usize, col_no: usize, max_cols: usize) -> Cow<'_, str> {
        let line_indent = self.line_indent(line);
        let indent = line_indent.saturating_sub(col_no);
        let col_no = col_no.saturating_sub(line_indent);

        let idx = self.idx_from_pos(Cursor { line, col: col_no });

        let raw =  match self.content.get(idx.element).map(Element::as_ref) {
            Some(ElementRef::Pager(pager)) => {
                pager.get_raw_line(idx.offset, col_no, max_cols)
            }
            Some(ElementRef::Str(s)) => {
                Cow::Borrowed(s[idx.offset..].get_first_line(max_cols))
            }
            None => Cow::Owned(String::new()),
        };
        if indent == 0 {
            raw
        } else {
            let mut owned = raw.into_owned();
            owned.insert_str(0, &" ".repeat(indent));
            Cow::Owned(owned)
        }
    }

    fn get_folding_range(&self, line: usize, parent: bool) -> Option<(Range<usize>, usize)> {
        let idx = self.idx_from_pos(Cursor { line, col: 0 });
        match self.content.get(idx.element)?.as_ref() {
            ElementRef::Pager(pager) => {
                let base_line = line - idx.offset;
                pager.get_folding_range(idx.offset, parent)
                    .map(|(range, level)| (range.start + base_line..range.end + base_line, level))
            }
            ElementRef::Str(_) => None,
        }
    }

    fn persist_line_number(&self, line: usize) -> (Vec<Anchor>, usize) {
        let idx = self.idx_from_pos(Cursor { line, col: 0 });
        match self.content.get(idx.element).map(Element::as_ref) {
            Some(ElementRef::Pager(pager)) => {
                let (mut anchor, line_offset) = pager.persist_line_number(idx.offset);
                anchor.push(Anchor::USize(idx.element));
                (anchor, line_offset)
            }
            _ => (vec![Anchor::USize2(idx.element, idx.offset)], 0)
        }
    }

    fn retrieve_line_number(&self, anchor: &[Anchor], line_offset: usize) -> (usize, bool) {
        let Some((discr, anchor)) = anchor.split_last() else {
            return (0, false);
        };
        let (element, offset) =
            match *discr {
                Anchor::USize(element) => (element, None),
                Anchor::USize2(element, offset) if line_offset == 0 => (element, Some(offset)),
                _ => return (0, false),
            };

        if let Some(offset) = offset {
            if let Some(ElementRef::Str(_)) = self.content.get(element).map(Element::as_ref) {
                return (self.pos_from_idx(Index { element, offset }).line, true)
            }
        } else {
            if let Some(ElementRef::Pager(pager)) = self.content.get(element).map(Element::as_ref) {
                let base_line = self.pos_from_idx(Index { element, offset: 0 }).line;
                let (line, success) = pager.retrieve_line_number(anchor, line_offset);
                return (base_line + line, success);
            }
        }

        if element >= self.content.len() {
            return (self.num_lines(), false);
        }

        (self.pos_from_idx(Index { element, offset: 0 }).line + line_offset, false)
    }
}
