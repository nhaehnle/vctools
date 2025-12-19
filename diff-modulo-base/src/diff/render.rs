// SPDX-License-Identifier: MIT

use super::file::FileName;
use super::hunks::HunkLine;

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
        old_begin: u32, // 1-based line number
        old_count: u32,
        new_begin: u32, // 1-based line number
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
                if line.contents.last().is_none_or(|ch| *ch != b'\n') {
                    out.extend(b"\n\\ No newline at end of file\n");
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
