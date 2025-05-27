
use std::cell::Cell;
use itertools::Itertools;

trait State<T> {
    fn get(&self) -> T;
    fn set(&mut self, t: T);
}
impl<T: Copy> State<T> for Cell<T> {
    fn get(&self) -> T {
        Cell::get(&self)
    }

    fn set(&mut self, t: T) {
        Cell::set(self, t);
    }
}
impl<T: Copy> State<T> for &mut T {
    fn get(&self) -> T {
        **self
    }

    fn set(&mut self, t: T) {
        **self = t;
    }
}

struct StrScanner<P, I> {
    pos: P,
    iter: I,
}
impl<P: State<(usize, usize)>, I: Iterator<Item = (usize, char)>> Iterator for StrScanner<P, I> {
    type Item = ((usize, usize), usize);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(byte_offset, ch)| {
            let orig_pos = self.pos.get();
            let (mut row, mut col) = self.pos.get();
            col += 1;
            if ch == '\n' {
                row += 1;
                col = 0;
            }
            self.pos.set((row, col));
            (orig_pos, byte_offset)
        })
    }
}

pub trait StrScan {
    fn row_col_scan(
        &self,
        init_pos: (usize, usize),
    ) -> impl Iterator<Item = ((usize, usize), usize)>;
    fn row_col_scan_mut(
        &self,
        pos: &mut (usize, usize),
    ) -> impl Iterator<Item = ((usize, usize), usize)>;
    fn get_first_line(&self, max_cols: usize) -> &str;
}
impl StrScan for str {
    /// Creates an iterator over ((line, column), byte_offset) tuples.
    ///
    /// `init_pos` is the (line, column) position of the start of the string.
    fn row_col_scan(
        &self,
        init_pos: (usize, usize),
    ) -> impl Iterator<Item = ((usize, usize), usize)> {
        StrScanner {
            pos: Cell::new(init_pos),
            iter: self.char_indices(),
        }
    }

    /// Creates an iterator over ((line, column), byte_offset) tuples.
    ///
    /// The initial value of `pos` is the (line, column) position of the start of the string.
    /// The iterator will update `pos` as it scans through the string.
    fn row_col_scan_mut(
        &self,
        pos: &mut (usize, usize),
    ) -> impl Iterator<Item = ((usize, usize), usize)> {
        StrScanner {
            pos,
            iter: self.char_indices(),
        }
    }

    /// Returns the first line of the string, truncated to `max_chars` characters.
    fn get_first_line(&self, max_chars: usize) -> &str {
        let bytes = self
            .char_indices()
            .take_while_inclusive(|&(_, ch)| ch != '\n')
            .take(max_chars.saturating_add(1))
            .last()
            .map_or(0, |(byte_offset, _)| byte_offset);

        &self[0..bytes]
    }
}
