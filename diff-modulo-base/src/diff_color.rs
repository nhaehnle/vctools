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

pub struct Writer {
    elements: Vec<Element>,
    rdm_column_widths: (usize, usize, usize, usize),
}
impl Writer {
    pub fn new() -> Writer {
        Writer {
            elements: Vec::new(),
            rdm_column_widths: (1, 1, 1, 1),
        }
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
            DiffChunkContents::FileHeader {
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
            DiffChunkContents::HunkHeader {
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
            DiffChunkContents::Line { line } => {
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
        struct Column(usize, Option<String>);
        impl std::fmt::Display for Column {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match &self.1 {
                    Some(string) => write!(f, "{string:0$}", self.0),
                    None => write!(f, "{0:-<1$}", '-', self.0),
                }
            }
        }

        out.set_color(
            ColorSpec::new()
                .set_bg(Some(Color::Cyan))
                .set_fg(Some(Color::Black)),
        )?;

        let change = match (rdm.changed, &rdm.old, &rdm.new) {
            (false, _, _) => "=",
            (true, Some(_), None) => "<",
            (true, None, Some(_)) => ">",
            _ => "!",
        };

        let (old_idx, old_hash) = rdm.old.as_ref().map_or((None, None), |(idx, hash)| {
            (Some(format!("{idx}")), Some(format!("{hash}")))
        });
        let (new_idx, new_hash) = rdm.new.as_ref().map_or((None, None), |(idx, hash)| {
            (Some(format!("{idx}")), Some(format!("{hash}")))
        });

        writeln!(
            out,
            "{}: {} {} {}: {} {}",
            Column(self.rdm_column_widths.0, old_idx),
            Column(self.rdm_column_widths.1, old_hash),
            change,
            Column(self.rdm_column_widths.2, new_idx),
            Column(self.rdm_column_widths.3, new_hash),
            String::from_utf8_lossy(&rdm.title)
        )?;

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
        let old_widths = rdm.old.as_ref().map_or((0, 0), |(idx, hash)| {
            (format!("{idx}").len(), format!("{hash}").len())
        });
        let new_widths = rdm.new.as_ref().map_or((0, 0), |(idx, hash)| {
            (format!("{idx}").len(), format!("{hash}").len())
        });

        self.rdm_column_widths.0 = self.rdm_column_widths.0.max(old_widths.0);
        self.rdm_column_widths.1 = self.rdm_column_widths.1.max(old_widths.1);
        self.rdm_column_widths.2 = self.rdm_column_widths.2.max(new_widths.0);
        self.rdm_column_widths.3 = self.rdm_column_widths.3.max(new_widths.1);

        self.elements.push(Element::RangeDiffMatch(rdm));
    }
}
