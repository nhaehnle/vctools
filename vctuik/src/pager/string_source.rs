// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

pub struct StringPagerSource<'text> {
    text: Cow<'text, str>,

    /// ((line number, column number), byte offset into text)
    /// Last entry is at end of text
    anchors: Vec<((usize, usize), usize)>,
}
impl<'text> StringPagerSource<'text> {
    pub fn new(text: impl Into<Cow<'text, str>>) -> Self {
        Self::do_new(text.into())
    }

    fn do_new(text: Cow<'text, str>) -> Self {
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

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line<'_> {
        let start = self.get_index(line, col_no);
        Line::from(self.text[start..].get_first_line(max_cols)).style(theme.normal)
    }

    fn get_raw_line(&self, line: usize, col_no: usize, max_cols: usize) -> Cow<'_, str> {
        let start = self.get_index(line, col_no);
        Cow::Borrowed(self.text[start..].get_first_line(max_cols))
    }

    fn persist_line_number(&self, line: usize) -> (Vec<Anchor>, usize) {
        (vec![], line)
    }

    fn retrieve_line_number(&self, anchor: &[Anchor], line_offset: usize) -> (usize, bool) {
        if !anchor.is_empty() {
            (0, false)
        } else {
            (line_offset, true)
        }
    }
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
