// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::str;
use std::vec::Vec;

use lazy_static::lazy_static;
use regex::bytes::Regex;

use crate::utils::*;

mod reduce_changed;
pub use reduce_changed::{reduce_changed_diff, reduce_changed_file, DiffAlgorithm};

/// A reference to a span of bytes in a [`Buffer`].
#[derive(Clone, Copy, Debug)]
pub struct DiffRef {
    begin: u32,
    end: u32,
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

    pub fn insert(&mut self, data: &[u8]) -> Result<DiffRef> {
        if data.len() >= u32::MAX as usize - self.buf.len() {
            return Err("Diffs larger than 4GB are not supported".into());
        }

        let begin = self.buf.len();
        self.buf.extend(data);
        Ok(DiffRef {
            begin: begin as u32,
            end: self.buf.len() as u32,
        })
    }

    /// Iterate over the lines of the buffer as [`DiffRef`]s spanning the line
    /// contents but not the new line character.
    fn lines(&self, range: DiffRef) -> LineIterator<'_> {
        assert!(range.begin <= range.end);
        assert!(range.end as usize <= self.buf.len());
        LineIterator {
            buffer: &self,
            range,
        }
    }
}

/// Represent an effective filename in a diff (without any prefix path
/// components). Missing means that the file is missing on the corresponding
/// side of the diff.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileName {
    Missing,
    Name(Vec<u8>),
}
impl Default for FileName {
    fn default() -> Self {
        FileName::Missing
    }
}
impl FileName {
    fn extract(path: &[u8], strip_path_components: usize) -> Result<FileName> {
        if path == b"/dev/null" {
            return Ok(Self::Missing);
        }

        if path.is_empty() {
            return Err("empty diff file path".into());
        }

        try_forward(
            || -> Result<_> {
                let mut path = path;
                if path[0] == b'/' {
                    path = &path[1..];
                }

                for _ in 0..strip_path_components {
                    path = match path
                        .iter()
                        .enumerate()
                        .find(|(_, &b)| b == b'/')
                        .map(|(idx, _)| idx)
                    {
                        Some(idx) => &path[idx + 1..],
                        None => {
                            return Err("path does not have enough components".into());
                        }
                    };
                }

                Ok(Self::Name(path.into()))
            },
            || String::from_utf8_lossy(path),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub enum HunkLineStatus {
    Unchanged,
    Old(bool), // "unimportant" boolean
    New(bool), // "unimportant" boolean
}
impl HunkLineStatus {
    pub fn symbol_byte(self) -> u8 {
        match self {
            HunkLineStatus::Unchanged => b' ',
            HunkLineStatus::Old(false) => b'-',
            HunkLineStatus::New(false) => b'+',
            HunkLineStatus::Old(true) => b'<',
            HunkLineStatus::New(true) => b'>',
        }
    }

    pub fn covers_old(self) -> bool {
        match self {
            HunkLineStatus::Unchanged | HunkLineStatus::Old(_) => true,
            HunkLineStatus::New(_) => false,
        }
    }

    pub fn covers_new(self) -> bool {
        match self {
            HunkLineStatus::Unchanged | HunkLineStatus::New(_) => true,
            HunkLineStatus::Old(_) => false,
        }
    }

    pub fn important(self) -> bool {
        match self {
            HunkLineStatus::Unchanged => false,
            HunkLineStatus::Old(unimportant) | HunkLineStatus::New(unimportant) => !unimportant,
        }
    }

    fn counts<I>(iter: I) -> (u32, u32)
    where
        I: Iterator<Item = HunkLineStatus>,
    {
        let mut old_count = 0;
        let mut new_count = 0;
        for status in iter {
            if status.covers_old() {
                old_count += 1;
            }
            if status.covers_new() {
                new_count += 1;
            }
        }
        (old_count, new_count)
    }
}

#[derive(Debug, Clone)]
pub struct HunkLine {
    pub status: HunkLineStatus,
    pub contents: Vec<u8>,
    pub no_newline: bool,
}
impl HunkLine {
    fn from_range<'a>(
        buffer: &'a Buffer,
        status: HunkLineStatus,
        lines: &'a [DiffRef],
    ) -> impl IntoIterator<Item = HunkLine> + 'a {
        lines.iter().map(move |&line| HunkLine {
            status,
            contents: buffer[line].to_vec(),
            no_newline: false,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Unknown,
    CommitMessage,
    Baseline,
    Change,
}
impl Context {
    pub fn prefix_bytes(self) -> &'static [u8] {
        match self {
            Context::Baseline => b"#",
            Context::Change => b" ",
            _ => &[],
        }
    }
}

#[derive(Debug, Clone)]
pub enum ChunkContents {
    FileHeader {
        /// Old path as found in the "---" line
        old_path: Vec<u8>,

        /// Old name, taking /dev/null and strip_path_components into account.
        old_name: FileName,

        /// New path as found in the "+++" line
        new_path: Vec<u8>,

        /// New name, taking /dev/null and strip_path_components into account.
        new_name: FileName,
    },
    HunkHeader {
        old_begin: u32,
        old_count: u32,
        new_begin: u32,
        new_count: u32,
    },
    Line {
        line: HunkLine,
    },
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub context: Context,
    pub contents: ChunkContents,
}
impl Chunk {
    pub fn render_text(&self, out: &mut Vec<u8>) {
        let prefix = self.context.prefix_bytes();

        match &self.contents {
            ChunkContents::FileHeader {
                old_path, new_path, ..
            } => {
                out.extend(prefix);
                out.extend(b"--- ");
                out.extend(old_path);
                out.push(b'\n');
                out.extend(prefix);
                out.extend(b"+++ ");
                out.extend(new_path);
                out.push(b'\n');
            }
            ChunkContents::HunkHeader {
                old_begin,
                old_count,
                new_begin,
                new_count,
            } => {
                out.extend(prefix);
                out.extend(
                    format!(
                        "@@ -{},{} +{},{} @@\n",
                        old_begin, old_count, new_begin, new_count
                    )
                    .as_bytes(),
                );
            }
            ChunkContents::Line { line } => {
                out.extend(prefix);
                out.push(line.status.symbol_byte());
                out.extend(&line.contents);
                if line.no_newline {
                    out.extend(b"\n\\ No newline at end of file\n");
                } else {
                    out.push(b'\n');
                }
            }
        }
    }
}

/// An object that receives [`Chunk`]s, e.g. to write them to a text file.
pub trait ChunkWriter {
    fn push_chunk(&mut self, chunk: Chunk);
}

pub trait ChunkWriterExt: ChunkWriter {
    /// Return a writer that forces all chunks into the given context before passing them on
    /// to the original (self) writer.
    fn with_context(&mut self, context: Context) -> impl ChunkWriter;
}
impl<T: ChunkWriter + ?Sized> ChunkWriterExt for T {
    fn with_context(&mut self, context: Context) -> impl ChunkWriter {
        struct WithContext<'writer, U: ?Sized> {
            this: &'writer mut U,
            context: Context,
        }
        impl<'writer, U: ChunkWriter + ?Sized> ChunkWriter for WithContext<'writer, U> {
            fn push_chunk(&mut self, chunk: Chunk) {
                let mut chunk = chunk;
                chunk.context = self.context;
                self.this.push_chunk(chunk);
            }
        }
        WithContext {
            this: self,
            context,
        }
    }
}

/// Write [`Chunk`]s into a byte buffer.
pub struct ChunkByteBufferWriter {
    pub out: Vec<u8>,
}
impl ChunkByteBufferWriter {
    pub fn new() -> Self {
        Self { out: Vec::new() }
    }
}
impl ChunkWriter for ChunkByteBufferWriter {
    fn push_chunk(&mut self, chunk: Chunk) {
        chunk.render_text(&mut self.out);
    }
}

struct LineIterator<'a> {
    buffer: &'a Buffer,
    range: DiffRef,
}

impl<'a> Iterator for LineIterator<'a> {
    type Item = DiffRef;

    fn next(&mut self) -> Option<DiffRef> {
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

            let result = DiffRef {
                begin: self.range.begin,
                end: line_end,
            };
            self.range.begin = next_begin;
            Some(result)
        }
    }
}

impl std::ops::Index<DiffRef> for Buffer {
    type Output = [u8];

    fn index(&self, index: DiffRef) -> &[u8] {
        &self.buf[(index.begin as usize)..(index.end as usize)]
    }
}

impl DiffRef {
    pub fn is_empty(&self) -> bool {
        self.begin >= self.end
    }

    pub fn len(&self) -> usize {
        if self.end > self.begin {
            (self.end - self.begin) as usize
        } else {
            0
        }
    }

    pub fn slice<R>(&self, range: R) -> DiffRef
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
        DiffRef {
            begin: self.begin + std::cmp::min(begin, self.len()) as u32,
            end: self.begin + std::cmp::min(end, self.len()) as u32,
        }
    }
}

impl Default for DiffRef {
    fn default() -> Self {
        DiffRef { begin: 0, end: 0 }
    }
}

#[derive(Debug, Clone)]
enum BlockContents {
    UnchangedUnknown(u32),
    UnchangedKnown(Vec<DiffRef>),
    Changed {
        old: Vec<DiffRef>,
        new: Vec<DiffRef>,
        unimportant: bool,
    },
    EndOfDiff {
        known_eof: bool,
        old_has_newline_at_eof: bool,
        new_has_newline_at_eof: bool,
    },
}
impl BlockContents {
    #[allow(unused)]
    fn is_unchanged_unknown(&self) -> bool {
        match self {
            BlockContents::UnchangedUnknown(_) => true,
            _ => false,
        }
    }

    fn is_unchanged_known(&self) -> bool {
        match self {
            BlockContents::UnchangedKnown(_) => true,
            _ => false,
        }
    }

    fn is_changed(&self) -> bool {
        match self {
            BlockContents::Changed { .. } => true,
            _ => false,
        }
    }

    fn is_end_of_diff(&self) -> bool {
        match self {
            BlockContents::EndOfDiff { .. } => true,
            _ => false,
        }
    }
}

/// The fundamental unit of the default diff representation: a block of old and
/// new lines.
#[derive(Debug, Clone)]
struct Block {
    old_begin: u32,
    new_begin: u32,
    contents: BlockContents,
}
impl Block {
    fn is_end_of_diff(&self) -> bool {
        self.contents.is_end_of_diff()
    }
}

#[derive(Debug, Clone)]
struct Hunk {
    old_begin: u32,
    new_begin: u32,
    lines: Vec<HunkLine>,
}
impl Hunk {
    fn counts(&self) -> (u32, u32) {
        HunkLineStatus::counts(self.lines.iter().map(|line| line.status))
    }

    fn render(&self, header: bool, writer: &mut dyn ChunkWriter) {
        if header {
            // TODO: Correct hunk header when one of old/new is an empty file
            let (old_count, new_count) = self.counts();
            writer.push_chunk(Chunk {
                context: Context::Unknown,
                contents: ChunkContents::HunkHeader {
                    old_begin: self.old_begin,
                    old_count,
                    new_begin: self.new_begin,
                    new_count,
                },
            });
        }

        for line in &self.lines {
            writer.push_chunk(Chunk {
                context: Context::Unknown,
                contents: ChunkContents::Line { line: line.clone() },
            });
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    // Old path as found in the "---" line
    old_path: Vec<u8>,

    // Old name, taking /dev/null and strip_path_components into account.
    pub old_name: FileName,

    /// New path as found in the "+++" line
    new_path: Vec<u8>,

    /// New name, taking /dev/null and strip_path_components into account.
    pub new_name: FileName,

    blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
struct Hunkify<'a> {
    buffer: &'a Buffer,
    blocks: &'a [Block],
    num_context_lines: Option<usize>,
    hunk: Hunk,
    important_end: usize,
}
impl<'a> Hunkify<'a> {
    fn set_location(&mut self, old: u32, new: u32) {
        if self.hunk.lines.is_empty() {
            self.hunk.old_begin = old;
            self.hunk.new_begin = new;
        } else {
            assert!({
                let (old_count, new_count) = self.hunk.counts();
                (old, new)
                    == (
                        self.hunk.old_begin + old_count,
                        self.hunk.new_begin + new_count,
                    )
            });
        }
    }

    fn add_unimportant(&mut self, status: HunkLineStatus, lines: &[DiffRef]) -> Option<Hunk> {
        if self.num_context_lines.is_none() {
            self.hunk
                .lines
                .extend(HunkLine::from_range(self.buffer, status, lines));
            return None;
        }

        let Some(num_context_lines) = self.num_context_lines else {
            panic!()
        };

        let current_context = self.hunk.lines.len() - self.important_end;
        let mut result = None;

        let mut taken = 0;

        if self.important_end != 0 {
            let missing_context = num_context_lines.saturating_sub(current_context);

            // Extend the trailing context to at most twice the requested number of context
            // lines. This is so we don't split hunks with changed lines separated by at most
            // twice the context.
            if lines.len() <= missing_context + num_context_lines {
                self.hunk
                    .lines
                    .extend(HunkLine::from_range(self.buffer, status, lines));
            } else {
                self.hunk.lines.extend(HunkLine::from_range(
                    self.buffer,
                    status,
                    &lines[..missing_context],
                ));
                taken += missing_context;

                result = self.flush_hunk();
            }
        }

        if self.important_end == 0 {
            let count = std::cmp::min(lines.len(), num_context_lines);
            let excess = (self.hunk.lines.len() + count).saturating_sub(num_context_lines);
            let (excess_old, excess_new) =
                HunkLineStatus::counts(self.hunk.lines.drain(..excess).map(|line| line.status));
            self.hunk.old_begin += excess_old;
            self.hunk.new_begin += excess_new;

            self.hunk.lines.extend(HunkLine::from_range(
                self.buffer,
                status,
                &lines[lines.len() - count..],
            ));
            taken += count;

            if status.covers_old() {
                self.hunk.old_begin += (lines.len() - taken) as u32;
            }
            if status.covers_new() {
                self.hunk.new_begin += (lines.len() - taken) as u32;
            }
        }

        result
    }

    fn add_important(&mut self, status: HunkLineStatus, lines: &[DiffRef]) {
        self.hunk
            .lines
            .extend(HunkLine::from_range(self.buffer, status, lines));
        self.important_end = self.hunk.lines.len();
    }

    fn add_unchanged(&mut self, lines: &[DiffRef]) -> Option<Hunk> {
        self.add_unimportant(HunkLineStatus::Unchanged, lines)
    }

    fn add_old(&mut self, old: &[DiffRef], unimportant: bool) -> Option<Hunk> {
        if unimportant {
            self.add_unimportant(HunkLineStatus::Old(true), old)
        } else {
            self.add_important(HunkLineStatus::Old(false), old);
            None
        }
    }

    fn add_new(&mut self, new: &[DiffRef], unimportant: bool) -> Option<Hunk> {
        if unimportant {
            self.add_unimportant(HunkLineStatus::New(true), new)
        } else {
            self.add_important(HunkLineStatus::New(false), new);
            None
        }
    }

    fn flush_hunk(&mut self) -> Option<Hunk> {
        if self.num_context_lines.is_some() && self.important_end == 0 {
            self.hunk.lines.clear();
            return None;
        }

        let (old_count, new_count) = self.hunk.counts();

        if let Some(num_context_lines) = self.num_context_lines {
            self.hunk
                .lines
                .truncate(self.important_end + num_context_lines);
        }
        self.important_end = 0;

        let next = Hunk {
            old_begin: self.hunk.old_begin + old_count,
            new_begin: self.hunk.new_begin + new_count,
            lines: Vec::new(),
        };
        Some(std::mem::replace(&mut self.hunk, next))
    }
}

impl<'a> Iterator for Hunkify<'a> {
    type Item = Hunk;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.blocks.is_empty() {
            let block;
            (block, self.blocks) = self.blocks.split_first().unwrap();

            self.set_location(block.old_begin, block.new_begin);

            match &block.contents {
                BlockContents::UnchangedKnown(lines) => {
                    if let Some(hunk) = self.add_unchanged(lines) {
                        return Some(hunk);
                    }
                }
                BlockContents::Changed {
                    old,
                    new,
                    unimportant,
                } => {
                    let hunk1 = self.add_old(old, *unimportant);
                    let hunk2 = self.add_new(new, *unimportant);
                    assert!(hunk1.is_none() || hunk2.is_none());
                    if let Some(hunk) = hunk1.or(hunk2) {
                        return Some(hunk);
                    }
                }
                _ => {
                    if let BlockContents::EndOfDiff {
                        known_eof,
                        old_has_newline_at_eof,
                        new_has_newline_at_eof,
                    } = &block.contents
                    {
                        if *known_eof {
                            let mut mark_no_newline_old = !*old_has_newline_at_eof;
                            let mut mark_no_newline_new = !*new_has_newline_at_eof;
                            for line in self.hunk.lines.iter_mut().rev() {
                                if line.status.covers_old() && mark_no_newline_old {
                                    line.no_newline = true;
                                    mark_no_newline_old = false;
                                }
                                if line.status.covers_new() && mark_no_newline_new {
                                    line.no_newline = true;
                                    mark_no_newline_new = false;
                                }
                                if !mark_no_newline_old && !mark_no_newline_new {
                                    break;
                                }
                            }
                        }
                    }

                    if let Some(hunk) = self.flush_hunk() {
                        return Some(hunk);
                    }
                }
            }
        }
        assert!(self.hunk.lines.is_empty());
        assert!(self.important_end == 0);

        None
    }
}

impl DiffFile {
    pub fn render_header(&self, writer: &mut dyn ChunkWriter) {
        writer.push_chunk(Chunk {
            context: Context::Unknown,
            contents: ChunkContents::FileHeader {
                old_path: self.old_path.clone(),
                old_name: self.old_name.clone(),
                new_path: self.new_path.clone(),
                new_name: self.new_name.clone(),
            },
        });
    }

    pub fn render(&self, buffer: &Buffer, num_context_lines: usize, writer: &mut dyn ChunkWriter) {
        let mut printed_header = false;
        for hunk in self.hunks(buffer, Some(num_context_lines)) {
            if !printed_header {
                self.render_header(writer);
                printed_header = true;
            }
            hunk.render(true, writer);
        }
    }

    pub fn render_full_body(&self, buffer: &Buffer, writer: &mut dyn ChunkWriter) {
        let mut hunks = self.hunks(buffer, None).peekable();
        let mut is_first = true;
        while let Some(hunk) = hunks.next() {
            let header = !is_first || hunks.peek().is_some();
            hunk.render(header, writer);
            is_first = false;
        }
    }

    pub fn is_unchanged(&self) -> bool {
        self.blocks.iter().all(|block| !block.contents.is_changed())
    }

    /// Iterate over hunks of the diff appropriate for rendering.
    ///
    /// If `num_context_lines` is `None`, iterate over hunks covering all known
    /// lines in the diff, regardless of whether they are changed or not.
    ///
    /// Otherwise, hunks will be reduced to at most the given number of lines
    /// surrounding important changes.
    fn hunks<'a>(
        &'a self,
        buffer: &'a Buffer,
        num_context_lines: Option<usize>,
    ) -> impl Iterator<Item = Hunk> + 'a {
        Hunkify {
            buffer,
            blocks: &self.blocks,
            num_context_lines,
            hunk: Hunk {
                old_begin: 0,
                new_begin: 0,
                lines: Vec::new(),
            },
            important_end: 0,
        }
    }

    fn simplify(&mut self) {
        let mut pending: Option<Block> = None;
        for block in std::mem::take(&mut self.blocks) {
            let mut prev = None;
            if let Some(pending) = pending.take() {
                if (pending.contents.is_unchanged_known() && block.contents.is_unchanged_known())
                    || (pending.contents.is_changed() && block.contents.is_changed())
                {
                    prev = Some(pending);
                } else {
                    self.blocks.push(pending);
                }
            }

            match &block.contents {
                BlockContents::UnchangedKnown(lines) => {
                    if let Some(mut prev) = prev.take() {
                        let BlockContents::UnchangedKnown(prev_lines) = &mut prev.contents else {
                            panic!()
                        };
                        prev_lines.extend(lines);
                        pending = Some(prev);
                    } else {
                        pending = Some(block);
                    }
                }
                BlockContents::Changed {
                    old,
                    new,
                    unimportant,
                } => {
                    if let Some(mut prev) = prev.take() {
                        let BlockContents::Changed {
                            old: prev_old,
                            new: prev_new,
                            unimportant: prev_unimportant,
                        } = &mut prev.contents
                        else {
                            panic!()
                        };
                        prev_old.extend(old);
                        prev_new.extend(new);
                        *prev_unimportant = *prev_unimportant && *unimportant;
                        pending = Some(prev);
                    } else {
                        pending = Some(block);
                    }
                }
                _ => {
                    self.blocks.push(block);
                }
            }

            assert!(prev.is_none());
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffOptions {
    pub strip_path_components: usize,
    pub num_context_lines: usize,
}
impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            strip_path_components: 1,
            num_context_lines: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diff {
    files: Vec<DiffFile>,
    options: DiffOptions,
}

impl Diff {
    pub fn new(options: DiffOptions) -> Self {
        Self {
            files: Vec::new(),
            options,
        }
    }

    pub fn add_file(&mut self, file: DiffFile) {
        self.files.push(file);
    }

    pub fn iter_files(&self) -> impl Iterator<Item = &DiffFile> + '_ {
        self.files.iter()
    }

    pub fn parse(buffer: &Buffer, range: DiffRef) -> Result<Diff> {
        #[derive(Default, Debug)]
        struct CurrentFile {
            old_path: Option<DiffRef>,
            old_name: Option<FileName>,
            new_path: Option<DiffRef>,
            new_name: Option<FileName>,
            blocks: Vec<Block>,
        }
        impl CurrentFile {
            fn prev_block_mut(&mut self) -> Option<&mut Block> {
                if self.blocks.len() >= 2 {
                    let idx = self.blocks.len() - 2;
                    Some(&mut self.blocks[idx])
                } else {
                    None
                }
            }
        }

        #[derive(Debug)]
        struct CurrentHunk {
            old_line: u32,
            new_line: u32,
            old_remaining: u32,
            new_remaining: u32,
        }

        struct DiffParser {
            diff_files: Vec<DiffFile>,
            file: Option<CurrentFile>,
            hunk: Option<CurrentHunk>,
            hunk_line: Option<DiffRef>,
        }
        impl DiffParser {
            fn ensure_file(&mut self) -> &mut CurrentFile {
                self.file.get_or_insert_with(|| CurrentFile {
                    blocks: Vec::from([Block {
                        old_begin: 1,
                        new_begin: 1,
                        contents: BlockContents::EndOfDiff {
                            known_eof: false,
                            old_has_newline_at_eof: true,
                            new_has_newline_at_eof: true,
                        },
                    }]),
                    ..CurrentFile::default()
                })
            }

            fn process_hunk_line(
                &mut self,
                buffer: &Buffer,
                lineref: DiffRef,
                no_newline_at_eof: bool,
            ) -> Result<()> {
                if lineref.len() < 1 {
                    return Err("completely empty hunk line".into());
                }

                let Some(file) = &mut self.file else { panic!() };
                let Some(hunk) = &mut self.hunk else { panic!() };

                let ch = buffer[lineref][0];
                let (is_old, is_new) = match ch {
                    b' ' => (true, true),
                    b'-' => (true, false),
                    b'+' => (false, true),
                    _ => return Err("noise found inside hunk".into()),
                };

                if is_old && is_new {
                    let prev = if let Some(
                        block @ Block {
                            contents: BlockContents::UnchangedKnown(_),
                            ..
                        },
                    ) = file.prev_block_mut()
                    {
                        block
                    } else {
                        file.blocks.insert(
                            file.blocks.len() - 1,
                            Block {
                                old_begin: hunk.old_line,
                                new_begin: hunk.new_line,
                                contents: BlockContents::UnchangedKnown(Vec::new()),
                            },
                        );
                        let idx = file.blocks.len() - 2;
                        &mut file.blocks[idx]
                    };
                    let BlockContents::UnchangedKnown(lines) = &mut prev.contents else {
                        panic!()
                    };
                    assert!(prev.old_begin + (lines.len() as u32) == hunk.old_line);
                    assert!(prev.new_begin + (lines.len() as u32) == hunk.new_line);
                    lines.push(lineref.slice(1..));
                } else {
                    assert!(is_old || is_new);
                    let prev = if let Some(
                        block @ Block {
                            contents: BlockContents::Changed { .. },
                            ..
                        },
                    ) = file.prev_block_mut()
                    {
                        block
                    } else {
                        file.blocks.insert(
                            file.blocks.len() - 1,
                            Block {
                                old_begin: hunk.old_line,
                                new_begin: hunk.new_line,
                                contents: BlockContents::Changed {
                                    old: Vec::new(),
                                    new: Vec::new(),
                                    unimportant: false,
                                },
                            },
                        );
                        let idx = file.blocks.len() - 2;
                        &mut file.blocks[idx]
                    };
                    let BlockContents::Changed { old, new, .. } = &mut prev.contents else {
                        panic!()
                    };
                    assert!(prev.old_begin + (old.len() as u32) == hunk.old_line);
                    assert!(prev.new_begin + (new.len() as u32) == hunk.new_line);
                    if is_old {
                        old.push(lineref.slice(1..));
                    } else {
                        new.push(lineref.slice(1..));
                    }
                }

                let last = file.blocks.last_mut().unwrap();
                assert!(last.old_begin == hunk.old_line);
                assert!(last.new_begin == hunk.new_line);

                if is_old {
                    hunk.old_line += 1;
                    last.old_begin += 1;
                    if hunk.old_remaining == 0 {
                        return Err("too many old lines in hunk".into());
                    }
                    hunk.old_remaining -= 1;

                    if no_newline_at_eof {
                        if hunk.old_remaining != 0 {
                            return Err("missing newline at EOF marker not at end of hunk".into());
                        }
                        let BlockContents::EndOfDiff {
                            known_eof,
                            old_has_newline_at_eof,
                            ..
                        } = &mut last.contents
                        else {
                            panic!()
                        };
                        *old_has_newline_at_eof = false;
                        *known_eof = true;
                    }
                }
                if is_new {
                    hunk.new_line += 1;
                    last.new_begin += 1;
                    if hunk.new_remaining == 0 {
                        return Err("too many new lines in hunk".into());
                    }
                    hunk.new_remaining -= 1;

                    if no_newline_at_eof {
                        if hunk.new_remaining != 0 {
                            return Err("missing newline at EOF marker not at end of hunk".into());
                        }
                        let BlockContents::EndOfDiff {
                            known_eof,
                            new_has_newline_at_eof,
                            ..
                        } = &mut last.contents
                        else {
                            panic!()
                        };
                        *new_has_newline_at_eof = false;
                        *known_eof = true;
                    }
                }

                if hunk.old_remaining == 0 && hunk.new_remaining == 0 {
                    self.hunk = None;
                }

                Ok(())
            }
        }

        let mut diff_options = DiffOptions::default();
        let mut parser = DiffParser {
            diff_files: Vec::new(),
            file: None,
            hunk: None,
            hunk_line: None,
        };

        let guard = [DiffRef::default()].into_iter();
        for (lineidx, lineref) in buffer.lines(range).chain(guard).enumerate() {
            try_forward(
                || -> Result<()> {
                    let line = &buffer[lineref];

                    if let Some(hunk_line) = parser.hunk_line.take() {
                        let no_newline = line == b"\\ No newline at end of file";

                        parser.process_hunk_line(&buffer, hunk_line, no_newline)?;
                        if no_newline {
                            return Ok(());
                        }
                    }

                    if parser.hunk.is_some() {
                        parser.hunk_line = Some(lineref);
                        return Ok(());
                    }

                    if line.starts_with(b"@@ ") {
                        let Some(file) = &mut parser.file else {
                            return Err("hunk without open file".into());
                        };

                        lazy_static! {
                            static ref RE: Regex =
                                Regex::new(r"(?-u)-(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
                                    .unwrap();
                        }
                        let captures = RE
                            .captures(&line[3..])
                            .ok_or_else(|| err_from_str("bad @@ line"))?;

                        fn get_u32(
                            captures: &regex::bytes::Captures,
                            idx: usize,
                            descr: &'static str,
                        ) -> Result<Option<u32>> {
                            try_forward(
                                || -> Result<Option<u32>> {
                                    Ok(match captures.get(idx) {
                                        Some(capture) => Some(
                                            str::from_utf8(capture.as_bytes())?.parse::<u32>()?,
                                        ),
                                        None => None,
                                    })
                                },
                                || descr,
                            )
                        }

                        let mut old_start = get_u32(&captures, 1, "old start")?.unwrap();
                        let old_count = get_u32(&captures, 2, "old count")?.unwrap_or(1);
                        let mut new_start = get_u32(&captures, 3, "new start")?.unwrap();
                        let new_count = get_u32(&captures, 4, "new count")?.unwrap_or(1);

                        let back_block = file.blocks.last_mut().unwrap();

                        let BlockContents::EndOfDiff { known_eof, .. } = &mut back_block.contents
                        else {
                            panic!()
                        };
                        if *known_eof {
                            return Err(
                                "hunk after definitive indication that EOF was reached".into()
                            );
                        }

                        if old_start == 0 {
                            if old_count != 0 {
                                return Err("surprising old line reference".into());
                            }
                            old_start = 1;
                            *known_eof = true;
                        }
                        if new_start == 0 {
                            if new_count != 0 {
                                return Err("surprising new line reference".into());
                            }
                            new_start = 1;
                            *known_eof = true;
                        }

                        if old_start < back_block.old_begin || new_start < back_block.new_begin {
                            return Err(
                                "hunks seem to be out of order or otherwise inconsistent?".into()
                            );
                        }

                        if old_start != back_block.old_begin || new_start != back_block.new_begin {
                            let count = old_start - back_block.old_begin;
                            if count != new_start - back_block.new_begin {
                                return Err("number of lines changed between hunks".into());
                            }
                            let filler = Block {
                                contents: BlockContents::UnchangedUnknown(count),
                                ..*back_block
                            };
                            back_block.old_begin = old_start;
                            back_block.new_begin = new_start;
                            file.blocks.insert(file.blocks.len() - 1, filler);
                        }

                        parser.hunk = Some(CurrentHunk {
                            old_line: old_start,
                            old_remaining: old_count,
                            new_line: new_start,
                            new_remaining: new_count,
                        });

                        return Ok(());
                    }

                    if let Some(file) = parser.file.take() {
                        if file.new_path.is_some() {
                            parser.diff_files.push(DiffFile {
                                old_path: buffer[file.old_path.unwrap()].to_vec(),
                                old_name: file.old_name.unwrap(),
                                new_path: buffer[file.new_path.unwrap()].to_vec(),
                                new_name: file.new_name.unwrap(),
                                blocks: file.blocks,
                            });
                        } else {
                            parser.file = Some(file);
                        }
                    }

                    if line.starts_with(b"--- ") {
                        let file = parser.ensure_file();
                        if file.old_path.is_some() {
                            return Err("multiple '---' lines found".into());
                        }

                        file.old_path = Some(lineref.slice(4..));
                        file.old_name = Some(FileName::extract(
                            &line[4..],
                            diff_options.strip_path_components,
                        )?);
                        return Ok(());
                    }
                    if line.starts_with(b"+++ ") {
                        let file = parser.ensure_file();
                        if file.old_path.is_none() {
                            return Err("found '+++' line without preceding '---' line".into());
                        }
                        if file.new_path.is_some() {
                            return Err("multiple '+++' lines found".into());
                        }

                        file.new_path = Some(lineref.slice(4..));
                        file.new_name = Some(FileName::extract(
                            &line[4..],
                            diff_options.strip_path_components,
                        )?);
                        return Ok(());
                    }

                    if parser.file.is_some() {
                        return Err("unrecognized noise in file".into());
                    }

                    // Just skip noise outside of a file region.
                    Ok(())
                },
                move || format!("line {}", lineidx + 1),
            )?;
        }

        if parser.hunk_line.is_some() {
            return Err("incomplete hunk at end of diff".into());
        }
        assert!(parser.file.is_none());

        // Estimate num_context_lines
        #[derive(Debug, PartialEq)]
        enum Predecessor {
            Boundary,
            Context(usize),
        }
        for file in &parser.diff_files {
            let mut pred = Predecessor::Boundary;
            for block in &file.blocks {
                pred = match &block.contents {
                    BlockContents::UnchangedKnown(lines) => {
                        if pred == Predecessor::Boundary {
                            diff_options.num_context_lines =
                                std::cmp::max(lines.len(), diff_options.num_context_lines);
                        }
                        Predecessor::Context(lines.len() as usize)
                    }
                    BlockContents::Changed { .. } => Predecessor::Context(0),
                    _ => {
                        if let Predecessor::Context(lines) = pred {
                            diff_options.num_context_lines =
                                std::cmp::max(lines, diff_options.num_context_lines);
                        }
                        Predecessor::Boundary
                    }
                };
            }
        }

        Ok(Diff {
            files: parser.diff_files,
            options: diff_options,
        })
    }

    pub fn render(&self, buffer: &Buffer, writer: &mut dyn ChunkWriter) {
        for file in &self.files {
            file.render(buffer, self.options.num_context_lines, writer);
        }
    }

    pub fn display_lossy<'a>(&'a self, buffer: &'a Buffer) -> LossyDiffDisplay<'a> {
        LossyDiffDisplay { diff: self, buffer }
    }

    /// Simplify the diff by merging adjacent blocks that are trivially mergable.
    pub fn simplify(&mut self) {
        for file in &mut self.files {
            file.simplify();
        }
    }
}

#[derive(Debug)]
pub struct LossyDiffDisplay<'a> {
    diff: &'a Diff,
    buffer: &'a Buffer,
}
impl<'a> std::fmt::Display for LossyDiffDisplay<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut writer = ChunkByteBufferWriter::new();
        self.diff.render(self.buffer, &mut writer);
        write!(f, "{}", String::from_utf8_lossy(&writer.out))
    }
}

/// An index over a diff file that allows quick lookup of files and hunks.
///
/// TODO: Actually implement this as an index?
#[derive(Debug)]
pub struct DiffIndex<'a> {
    diff: &'a Diff,
}

impl<'a> DiffIndex<'a> {
    pub fn create(diff: &'a Diff) -> Self {
        Self { diff }
    }

    pub fn find_old_file<'b, 'c>(&'b self, name: &'c FileName) -> Option<&'b DiffFile> {
        if *name == FileName::Missing {
            None
        } else {
            self.diff.files.iter().find(|file| file.old_name == *name)
        }
    }

    pub fn find_new_file<'b, 'c>(&'b self, name: &'c FileName) -> Option<&'b DiffFile> {
        if *name == FileName::Missing {
            None
        } else {
            self.diff.files.iter().find(|file| file.new_name == *name)
        }
    }

    pub fn is_range_changed(&self, file: &DiffFile, old: bool, begin: u32, end: u32) -> bool {
        let idx = file.blocks.partition_point(|block| {
            let block_begin = if old {
                block.old_begin
            } else {
                block.new_begin
            };
            block_begin <= end
        });

        for block in file.blocks[..idx].iter().rev() {
            if let BlockContents::Changed { .. } = &block.contents {
                return true;
            }
            let block_begin = if old {
                block.old_begin
            } else {
                block.new_begin
            };
            if block_begin < begin {
                return false;
            }
        }
        false
    }
}

/// Compute the diff that results from composing `first` followed by `second`.
///
/// Note: This function performs a trivial simplification of the diff but does
/// not look for opportunities for further simplification in case the second
/// diff (partially) reverts the first one. Doing so requires access to the
/// underlying buffer.
pub fn compose(first: &Diff, second: &Diff) -> Result<Diff> {
    if first.options.strip_path_components != second.options.strip_path_components {
        return Err("Don't know how to compose diffs with inconsistent path strip".into());
    }

    let mut result = Diff {
        files: Vec::new(),
        options: DiffOptions {
            strip_path_components: first.options.strip_path_components,
            num_context_lines: std::cmp::max(
                first.options.num_context_lines,
                second.options.num_context_lines,
            ),
        },
    };

    let first_diff_idx = DiffIndex::create(first);
    let second_diff_idx = DiffIndex::create(second);

    let mut recreated: HashSet<FileName> = HashSet::new();

    for first_file in &first.files {
        if let Some(second_file) = second_diff_idx.find_old_file(&first_file.new_name) {
            let mut file = DiffFile {
                old_path: first_file.old_path.clone(),
                old_name: first_file.old_name.clone(),
                new_path: second_file.new_path.clone(),
                new_name: second_file.new_name.clone(),
                blocks: Vec::new(),
            };
            let mut first_idx = 0;
            let mut second_idx = 0;
            let mut mid_line = 1;

            #[derive(Debug, Clone, Copy)]
            enum SplitContents<'a> {
                Unknown(u32),
                Known(&'a [DiffRef]),
            }
            impl<'a> SplitContents<'a> {
                fn len(&self) -> u32 {
                    match self {
                        SplitContents::Unknown(len) => *len,
                        SplitContents::Known(lines) => lines.len() as u32,
                    }
                }

                fn lines(&self) -> &'a [DiffRef] {
                    match self {
                        SplitContents::Known(lines) => lines,
                        SplitContents::Unknown(_) => panic!(),
                    }
                }
            }

            struct SplitBlock<'a> {
                change: Option<bool>, // bool: unimportant?
                old: SplitContents<'a>,
                new: SplitContents<'a>,
            }
            impl<'a> SplitBlock<'a> {
                fn new_unchanged(contents: SplitContents<'a>) -> Self {
                    Self {
                        change: None,
                        old: contents,
                        new: contents,
                    }
                }

                fn from(block_contents: &'a BlockContents) -> Self {
                    match block_contents {
                        BlockContents::UnchangedUnknown(len) => {
                            Self::new_unchanged(SplitContents::Unknown(*len))
                        }
                        BlockContents::UnchangedKnown(lines) => {
                            Self::new_unchanged(SplitContents::Known(&lines))
                        }
                        BlockContents::Changed {
                            old,
                            new,
                            unimportant,
                        } => Self {
                            change: Some(*unimportant),
                            old: SplitContents::Known(&old),
                            new: SplitContents::Known(&new),
                        },
                        BlockContents::EndOfDiff { .. } => {
                            Self::new_unchanged(SplitContents::Unknown(u32::MAX))
                        }
                    }
                }
            }

            loop {
                let first_block = &first_file.blocks[first_idx];
                let second_block = &second_file.blocks[second_idx];

                let first_mid_ofs = mid_line - first_block.new_begin;
                let second_mid_ofs = mid_line - second_block.old_begin;

                if let (
                    BlockContents::EndOfDiff {
                        known_eof: first_known_eof,
                        old_has_newline_at_eof: first_old_has_newline_at_eof,
                        new_has_newline_at_eof: first_new_has_newline_at_eof,
                    },
                    BlockContents::EndOfDiff {
                        known_eof: second_known_eof,
                        old_has_newline_at_eof: second_old_has_newline_at_eof,
                        new_has_newline_at_eof: second_new_has_newline_at_eof,
                    },
                ) = (&first_block.contents, &second_block.contents)
                {
                    file.blocks.push(Block {
                        old_begin: first_block.old_begin + first_mid_ofs,
                        new_begin: second_block.new_begin + second_mid_ofs,
                        contents: BlockContents::EndOfDiff {
                            known_eof: *first_known_eof || *second_known_eof,
                            old_has_newline_at_eof: if *first_known_eof {
                                *first_old_has_newline_at_eof
                            } else {
                                *second_old_has_newline_at_eof
                            },
                            new_has_newline_at_eof: if *second_known_eof {
                                *second_new_has_newline_at_eof
                            } else {
                                *first_new_has_newline_at_eof
                            },
                        },
                    });
                    break;
                }

                assert!(!first_block.is_end_of_diff() || !second_block.is_end_of_diff());

                let first_split = SplitBlock::from(&first_block.contents);
                let second_split = SplitBlock::from(&second_block.contents);

                let first_mid_remaining = first_split.new.len() - first_mid_ofs;
                let second_mid_remaining = second_split.old.len() - second_mid_ofs;
                let count = std::cmp::min(first_mid_remaining, second_mid_remaining);

                let all_first = count == first_mid_remaining;
                let all_second = count == second_mid_remaining;

                if first_split.change.is_none() && second_split.change.is_none() {
                    let lines = if let SplitContents::Known(lines) = first_split.old {
                        Some(&lines[first_mid_ofs as usize..])
                    } else if let SplitContents::Known(lines) = second_split.new {
                        Some(&lines[second_mid_ofs as usize..])
                    } else {
                        None
                    };

                    file.blocks.push(Block {
                        old_begin: first_block.old_begin + first_mid_ofs,
                        new_begin: second_block.new_begin + second_mid_ofs,
                        contents: match lines {
                            Some(lines) => {
                                BlockContents::UnchangedKnown(lines[..count as usize].into())
                            }
                            None => BlockContents::UnchangedUnknown(count),
                        },
                    });
                } else {
                    // At least one of the diffs is a genuine change. Insert a
                    // change block.
                    let old;
                    if first_split.change.is_some() {
                        if all_first {
                            old = first_split.old.lines().into();
                        } else {
                            old = Vec::new();
                        }
                    } else {
                        old = second_split.old.lines()
                            [second_mid_ofs as usize..(second_mid_ofs + count) as usize]
                            .into();
                    }

                    let new;
                    if second_split.change.is_some() {
                        if all_second {
                            new = second_split.new.lines().into();
                        } else {
                            new = Vec::new();
                        }
                    } else {
                        new = first_split.new.lines()
                            [first_mid_ofs as usize..(first_mid_ofs + count) as usize]
                            .into();
                    }

                    if !old.is_empty() || !new.is_empty() {
                        file.blocks.push(Block {
                            old_begin: first_block.old_begin
                                + if first_split.change.is_none() {
                                    first_mid_ofs
                                } else {
                                    0
                                },
                            new_begin: second_block.new_begin
                                + if second_split.change.is_none() {
                                    second_mid_ofs
                                } else {
                                    0
                                },
                            contents: BlockContents::Changed {
                                old,
                                new,
                                unimportant: first_split.change.unwrap_or(true)
                                    && second_split.change.unwrap_or(true),
                            },
                        });
                    }
                }

                mid_line += count;
                if all_first {
                    first_idx += 1;
                }
                if all_second {
                    second_idx += 1;
                }
            }

            result.files.push(file);
            continue;
        }

        // Find "recreated" files.
        //
        // TODO: Ideally, this would be able to track renames to some extent.
        if first_file.new_name == FileName::Missing {
            if let Some(second_file) = second_diff_idx.find_new_file(&first_file.old_name) {
                if second_file.old_name == FileName::Missing {
                    assert!(first_file.blocks.len() == 2);
                    assert!(second_file.blocks.len() == 2);

                    let BlockContents::Changed {
                        old,
                        unimportant: first_unimportant,
                        ..
                    } = &first_file.blocks[0].contents
                    else {
                        panic!()
                    };
                    let BlockContents::Changed {
                        new,
                        unimportant: second_unimportant,
                        ..
                    } = &second_file.blocks[0].contents
                    else {
                        panic!()
                    };

                    let BlockContents::EndOfDiff {
                        old_has_newline_at_eof,
                        ..
                    } = first_file.blocks[1].contents
                    else {
                        panic!()
                    };
                    let BlockContents::EndOfDiff {
                        new_has_newline_at_eof,
                        ..
                    } = second_file.blocks[1].contents
                    else {
                        panic!()
                    };

                    result.files.push(DiffFile {
                        old_path: first_file.old_path.clone(),
                        old_name: first_file.old_name.clone(),
                        new_path: second_file.new_path.clone(),
                        new_name: second_file.new_name.clone(),
                        blocks: [
                            Block {
                                old_begin: 1,
                                new_begin: 1,
                                contents: BlockContents::Changed {
                                    old: old.clone(),
                                    new: new.clone(),
                                    unimportant: *first_unimportant && *second_unimportant,
                                },
                            },
                            Block {
                                old_begin: first_file.blocks[1].old_begin,
                                new_begin: second_file.blocks[1].new_begin,
                                contents: BlockContents::EndOfDiff {
                                    known_eof: true,
                                    old_has_newline_at_eof,
                                    new_has_newline_at_eof,
                                },
                            },
                        ]
                        .into(),
                    });
                    recreated.insert(second_file.new_name.clone());
                    continue;
                }
            }
        }

        result.files.push(first_file.clone());
    }

    for second_file in &second.files {
        if second_file.old_name == FileName::Missing && recreated.contains(&second_file.new_name) {
            continue;
        }
        if first_diff_idx
            .find_new_file(&second_file.old_name)
            .is_none()
        {
            result.files.push(second_file.clone());
        }
    }

    result.simplify();
    Ok(result)
}

/// Compute the reverse diff.
pub fn reverse(diff: &Diff) -> Diff {
    let mut result = diff.clone();

    for file in &mut result.files {
        std::mem::swap(&mut file.old_path, &mut file.new_path);
        std::mem::swap(&mut file.old_name, &mut file.new_name);

        for block in &mut file.blocks {
            std::mem::swap(&mut block.old_begin, &mut block.new_begin);
            if let BlockContents::Changed { old, new, .. } = &mut block.contents {
                std::mem::swap(old, new);
            } else if let BlockContents::EndOfDiff {
                old_has_newline_at_eof,
                new_has_newline_at_eof,
                ..
            } = &mut block.contents
            {
                std::mem::swap(old_has_newline_at_eof, new_has_newline_at_eof);
            }
        }
    }

    result
}

/// Reduce the `target` diff based on knowledge about the `old` and `new` diffs.
pub fn reduce_modulo_base<'a>(
    mut target: Diff,
    target_is_base: bool,
    base_old: &'a Diff,
    base_new: &'a Diff,
) -> Result<Diff> {
    let old_index = DiffIndex::create(base_old);
    let new_index = DiffIndex::create(base_new);

    target.files = target
        .files
        .into_iter()
        .filter_map(|mut file| {
            let old_ref;
            let new_ref;
            if target_is_base {
                old_ref = old_index.find_old_file(&file.old_name);
                new_ref = new_index.find_old_file(&file.new_name);
            } else {
                old_ref = old_index.find_new_file(&file.old_name);
                new_ref = new_index.find_new_file(&file.new_name);
            }

            if old_ref.is_none() && new_ref.is_none() {
                // The file is affected by neither the base..old nor the base..new
                // diff. We should remove it entirely.
                return None;
            }

            for block in &mut file.blocks {
                if let BlockContents::Changed {
                    old,
                    new,
                    unimportant,
                } = &mut block.contents
                {
                    let mut important = false;
                    if let Some(old_ref) = old_ref {
                        if old_index.is_range_changed(
                            old_ref,
                            target_is_base,
                            block.old_begin,
                            block.old_begin + (old.len() as u32),
                        ) {
                            important = true;
                        }
                    }
                    if let Some(new_ref) = new_ref {
                        if new_index.is_range_changed(
                            new_ref,
                            target_is_base,
                            block.new_begin,
                            block.new_begin + (new.len() as u32),
                        ) {
                            important = true;
                        }
                    }
                    *unimportant = !important;
                }
            }

            Some(file)
        })
        .collect();

    Ok(target)
}

pub fn diff_modulo_base(
    buffer: &Buffer,
    target: Diff,
    base_old: &Diff,
    base_new: &Diff,
    writer: &mut dyn ChunkWriter,
) -> Result<()> {
    let base = compose(base_old, &target)?;
    let base = compose(&base, &reverse(base_new))?;
    let base = reduce_modulo_base(base, true, base_old, base_new)?;
    let base = reduce_changed_diff(buffer, base, DiffAlgorithm::default());

    let target = reduce_modulo_base(target, false, base_old, base_new)?;

    let base_old_index = DiffIndex::create(base_old);
    let base_new_index = DiffIndex::create(base_new);
    let target_index = DiffIndex::create(&target);
    let base_index = DiffIndex::create(&base);

    let num_context_lines = std::cmp::max(
        base.options.num_context_lines,
        target.options.num_context_lines,
    );

    for base_file in &base.files {
        let target_file = base_old_index
            .find_old_file(&base_file.old_name)
            .and_then(|base_old_file| target_index.find_old_file(&base_old_file.new_name))
            .or_else(|| {
                base_new_index
                    .find_old_file(&base_file.new_name)
                    .and_then(|base_new_file| target_index.find_new_file(&base_new_file.new_name))
            });
        if let Some(target_file) = target_file {
            let mut need_base_header = false;
            let mut need_target_header = false;

            let mut base_hunks = base_file.hunks(buffer, Some(num_context_lines)).peekable();
            let mut target_hunks = target_file
                .hunks(buffer, Some(num_context_lines))
                .peekable();

            let mut hunks: Vec<(Context, Hunk)> = Vec::new();

            loop {
                let base_hunk = base_hunks.peek();
                let target_hunk = target_hunks.peek();
                if base_hunk.is_none() && target_hunk.is_none() {
                    break;
                }

                let render_base;
                if let Some(target_hunk) = target_hunk {
                    if let Some(base_hunk) = base_hunk {
                        // TODO: Better algorithm for lining up base vs. target hunks
                        let (old_count, new_count) = target_hunk.counts();
                        render_base = base_hunk.old_begin <= target_hunk.old_begin + old_count
                            || base_hunk.new_begin <= target_hunk.new_begin + new_count;
                    } else {
                        render_base = false;
                    }
                } else {
                    render_base = true;
                }

                if render_base {
                    hunks.push((Context::Baseline, base_hunks.next().unwrap()));
                    need_base_header = true;
                } else {
                    hunks.push((Context::Change, target_hunks.next().unwrap()));
                    need_target_header = true;
                }
            }

            if need_base_header {
                base_file.render_header(&mut writer.with_context(Context::Baseline));
            }
            if need_target_header {
                target_file.render_header(&mut writer.with_context(Context::Change));
            }

            for (context, hunk) in hunks {
                hunk.render(true, &mut writer.with_context(context));
            }
        } else {
            base_file.render(
                buffer,
                num_context_lines,
                &mut writer.with_context(Context::Baseline),
            );
        }
    }

    for target_file in &target.files {
        let base_file = base_old_index
            .find_new_file(&target_file.old_name)
            .and_then(|base_old_file| base_index.find_old_file(&base_old_file.old_name))
            .or_else(|| {
                base_new_index
                    .find_new_file(&target_file.new_name)
                    .and_then(|base_new_file| base_index.find_new_file(&base_new_file.old_name))
            });
        if base_file.is_none() {
            target_file.render(
                buffer,
                num_context_lines,
                &mut writer.with_context(Context::Change),
            );
        }
    }

    Ok(())
}

pub fn diff_file(
    buffer: &Buffer,
    old_path: DiffRef,
    new_path: DiffRef,
    old_body: DiffRef,
    new_body: DiffRef,
    options: &DiffOptions,
    algorithm: DiffAlgorithm,
) -> Result<DiffFile> {
    let old_lines: Vec<DiffRef> = buffer.lines(old_body).collect();
    let new_lines: Vec<DiffRef> = buffer.lines(new_body).collect();
    let mut old_has_newline_at_eof = true;
    let mut new_has_newline_at_eof = true;
    if let Some(line) = old_lines.last() {
        if line.end == old_body.end {
            old_has_newline_at_eof = false;
        }
    }
    if let Some(line) = new_lines.last() {
        if line.end == new_body.end {
            new_has_newline_at_eof = false;
        }
    }

    let num_old_lines = old_lines.len() as u32;
    let num_new_lines = new_lines.len() as u32;

    let file = DiffFile {
        old_path: buffer[old_path].to_owned(),
        old_name: FileName::extract(&buffer[old_path], options.strip_path_components)?,
        new_path: buffer[new_path].to_owned(),
        new_name: FileName::extract(&buffer[new_path], options.strip_path_components)?,
        blocks: [
            Block {
                old_begin: 1,
                new_begin: 1,
                contents: BlockContents::Changed {
                    old: old_lines,
                    new: new_lines,
                    unimportant: false,
                },
            },
            Block {
                old_begin: 1 + num_old_lines,
                new_begin: 1 + num_new_lines,
                contents: BlockContents::EndOfDiff {
                    known_eof: true,
                    old_has_newline_at_eof,
                    new_has_newline_at_eof,
                },
            },
        ]
        .into(),
    };

    Ok(reduce_changed_file(buffer, file, algorithm).0)
}
