// SPDX-License-Identifier: MIT

///! [`Buffer`]s are unstructured storage of bytes that are used to hold
///! diff contents. All other data structures simply hold [`BufferRef`]s into
///! a common buffer.

use vctools_utils::prelude::*;

/// A reference to a span of bytes in a [`Buffer`].
#[derive(Clone, Copy, Debug)]
pub struct BufferRef {
    pub begin: u32,
    pub end: u32,
}
impl Default for BufferRef {
    fn default() -> Self {
        BufferRef { begin: 0, end: 0 }
    }
}
impl BufferRef {
    pub fn is_empty(&self) -> bool {
        self.begin >= self.end
    }

    pub fn len(&self) -> usize {
        (self.end - self.begin) as usize
    }

    pub fn slice<R>(&self, range: R) -> BufferRef
    where
        R: std::ops::RangeBounds<usize>,
    {
        use std::ops::Bound::*;

        let begin = match range.start_bound() {
            Included(&x) => x,
            Excluded(&x) => x.checked_add(1).unwrap_or(self.len()),
            Unbounded => 0,
        };
        let end = match range.end_bound() {
            Included(&x) => x.checked_add(1).unwrap_or(self.len()),
            Excluded(&x) => x,
            Unbounded => self.len(),
        };
        BufferRef {
            begin: self.begin + std::cmp::min(begin, self.len()) as u32,
            end: self.begin + std::cmp::min(end, self.len()) as u32,
        }
    }
}

/// Owner of diff contents.
#[derive(Debug)]
pub struct Buffer {
    buf: Vec<u8>,
}

impl Buffer {
    pub fn new() -> Self {
        Buffer { buf: Vec::new() }
    }

    pub fn insert(&mut self, data: &[u8]) -> Result<BufferRef> {
        if data.len() >= u32::MAX as usize - self.buf.len() {
            return Err("Diffs larger than 4GB are not supported".into());
        }

        let begin = self.buf.len();
        self.buf.extend(data);
        Ok(BufferRef {
            begin: begin as u32,
            end: self.buf.len() as u32,
        })
    }

    /// Iterate over the lines of the buffer as [`BufferRef`]s spanning the line
    /// contents but not the new line character.
    pub fn lines(&self, range: BufferRef) -> LineIterator<'_> {
        assert!(range.begin <= range.end);
        assert!(range.end as usize <= self.buf.len());
        LineIterator {
            buffer: &self,
            range,
        }
    }

    pub fn get(&self, at: u32) -> Option<u8> {
        self.buf.get(at as usize).copied()
    }
}

pub struct LineIterator<'a> {
    buffer: &'a Buffer,
    range: BufferRef,
}
impl<'a> Iterator for LineIterator<'a> {
    type Item = BufferRef;

    fn next(&mut self) -> Option<BufferRef> {
        if self.range.begin >= self.range.end {
            None
        } else {
            let (line_end, next_begin) = match self.buffer[self.range]
                .iter()
                .enumerate()
                .find(|(_, &b)| b == b'\n')
            {
                Some((idx, _)) => (
                    self.range.begin + idx as u32,
                    self.range.begin + idx as u32 + 1,
                ),
                None => (self.range.end, self.range.end),
            };

            let result = BufferRef {
                begin: self.range.begin,
                end: line_end,
            };
            self.range.begin = next_begin;
            Some(result)
        }
    }
}

impl std::ops::Index<BufferRef> for Buffer {
    type Output = [u8];

    fn index(&self, index: BufferRef) -> &[u8] {
        &self.buf[(index.begin as usize)..(index.end as usize)]
    }
}
impl std::ops::Index<u32> for Buffer {
    type Output = u8;

    fn index(&self, index: u32) -> &u8 {
        &self.buf[index as usize]
    }
}
