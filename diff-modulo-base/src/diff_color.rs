// SPDX-License-Identifier: MIT

use lazy_static::lazy_static;
use termcolor::{Color, ColorSpec};

use crate::*;
use diff::*;
use git_core::{RangeDiffMatch, RangeDiffWriter};

#[derive(Default)]
struct Colors {
    default: ColorSpec,
    file_header: ColorSpec,
    hunk_header: ColorSpec,
    baseline: ColorSpec,
    new_important: ColorSpec,
    new_unimportant: ColorSpec,
    old_important: ColorSpec,
    old_unimportant: ColorSpec,
}
impl Colors {
    fn new() -> Self {
        let mut colors = Colors {
            ..Default::default()
        };
        colors.file_header.set_bold(true);
        colors.hunk_header.set_fg(Some(Color::Cyan));
        colors.baseline.set_dimmed(true);
        colors.new_important.set_fg(Some(Color::Green));
        colors.new_unimportant.set_fg(Some(Color::Green));
        colors.old_important.set_fg(Some(Color::Red));
        colors.old_unimportant.set_fg(Some(Color::Red));
        colors
    }
}
lazy_static! {
    static ref COLORS: Colors = Colors::new();
}

fn get_line_color(context: Context, state: HunkLineStatus) -> &'static ColorSpec {
    match state {
        HunkLineStatus::New(unimportant) => {
            if unimportant || context != Context::Change {
                &COLORS.new_unimportant
            } else {
                &COLORS.new_important
            }
        }
        HunkLineStatus::Old(unimportant) => {
            if unimportant || context != Context::Change {
                &COLORS.old_unimportant
            } else {
                &COLORS.old_important
            }
        }
        HunkLineStatus::Unchanged => {
            if context == Context::Baseline {
                &COLORS.baseline
            } else {
                &COLORS.default
            }
        }
    }
}

#[derive(Debug)]
enum Element {
    Chunk(Chunk),
    RangeDiffMatch(RangeDiffMatch),
}

#[derive(Default)]
pub struct Writer {
    elements: Vec<Element>,
    rdm_column_widths: git_core::RangeDiffMatchColumnWidths,
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(mut self, out: &mut dyn termcolor::WriteColor) -> std::io::Result<()> {
        for element in std::mem::take(&mut self.elements) {
            match element {
                Element::Chunk(chunk) => {
                    self.write_chunk(out, chunk)?;
                }
                Element::RangeDiffMatch(rdm) => {
                    self.write_range_diff_match(out, rdm)?;
                }
            }
        }

        Ok(())
    }

    fn write_chunk(
        &self,
        out: &mut dyn termcolor::WriteColor,
        chunk: Chunk,
    ) -> std::io::Result<()> {
        let prefix = chunk.context.prefix_bytes();

        match &chunk.contents {
            ChunkContents::FileHeader {
                old_path, new_path, ..
            } => {
                out.set_color(&COLORS.file_header)?;
                out.write(prefix)?;
                out.write(b"--- ")?;
                out.write(old_path)?;
                out.write(b"\n")?;
                out.set_color(&COLORS.file_header)?;
                out.write(prefix)?;
                out.write(b"+++ ")?;
                out.write(new_path)?;
                out.write(b"\n")?;
            }
            ChunkContents::HunkHeader {
                old_begin,
                old_count,
                new_begin,
                new_count,
            } => {
                out.set_color(&COLORS.hunk_header)?;
                out.write(prefix)?;
                out.write(
                    format!(
                        "@@ -{},{} +{},{} @@\n",
                        old_begin, old_count, new_begin, new_count
                    )
                    .as_bytes(),
                )?;
                out.reset()?;
            }
            ChunkContents::Line { line } => {
                let color = get_line_color(chunk.context, line.status);
                if color != &COLORS.default {
                    out.set_color(color)?;
                }
                out.write(prefix)?;
                out.write(&[line.status.symbol_byte()])?;
                out.write(&line.contents)?;
                if line.no_newline {
                    out.write(b"\n\\ No newline at end of file\n")?;
                } else {
                    out.write(b"\n")?;
                }
            }
        }

        Ok(())
    }

    fn write_range_diff_match(
        &self,
        out: &mut dyn termcolor::WriteColor,
        rdm: RangeDiffMatch,
    ) -> std::io::Result<()> {
        out.set_color(
            ColorSpec::new()
                .set_bg(Some(Color::Cyan))
                .set_fg(Some(Color::Black)),
        )?;

        writeln!(out, "{}", rdm.format(self.rdm_column_widths))?;

        Ok(())
    }
}
impl ChunkWriter for Writer {
    fn push_chunk(&mut self, chunk: Chunk) {
        self.elements.push(Element::Chunk(chunk));
    }
}
impl RangeDiffWriter for Writer {
    fn push_range_diff_match(&mut self, rdm: RangeDiffMatch) {
        self.rdm_column_widths = self.rdm_column_widths.max(rdm.column_widths());

        self.elements.push(Element::RangeDiffMatch(rdm));
    }
}
