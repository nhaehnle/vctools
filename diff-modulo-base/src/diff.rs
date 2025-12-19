// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::str;
use std::vec::Vec;

#[allow(unused_imports)]
use itertools::Itertools;
use lazy_static::lazy_static;
use regex::bytes::Regex;

use crate::utils::*;

mod buffer;
mod file;
mod file_match;
mod hunks;
mod reduce_changed;
pub mod render;

pub use buffer::{Buffer, BufferRef};
use file::parse_diff_path;
pub use file::{File, FileBuilder, FileName};
pub use file_match::{FileMatch, MatchStatus, MatchStatusMarker};
pub use hunks::{hunkify, Hunk, HunkLine, HunkLineStatus};
pub use reduce_changed::{reduce_changed_diff, reduce_changed_file, DiffAlgorithm};

use render::ChunkWriterExt;

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
    files: Vec<FileMatch>,
    options: DiffOptions,
}

impl Diff {
    pub fn new(options: DiffOptions) -> Self {
        Self {
            files: Vec::new(),
            options,
        }
    }

    pub fn add_file(&mut self, file: FileMatch) {
        self.files.push(file);
    }

    pub fn iter_files(&self) -> impl Iterator<Item = &FileMatch> + '_ {
        self.files.iter()
    }

    pub fn parse(buffer: &Buffer, range: BufferRef) -> Result<Diff> {
        #[derive(Default, Debug)]
        struct CurrentFileMatch {
            old_path: Option<BufferRef>,
            old: Option<(BufferRef, FileBuilder)>,
            new_path: Option<BufferRef>,
            new: Option<(BufferRef, FileBuilder)>,
            known_eof: bool,
            known_eof_after_hunk: bool,
            status_markers: Vec<MatchStatusMarker>,
            old_line: u32,
            new_line: u32,
        }

        #[derive(Debug)]
        struct CurrentHunk {
            old_remaining: u32,
            new_remaining: u32,
            unchanged_count: u32, // # contiguous unchanged lines
            seen_changed: bool,
        }

        struct DiffParser {
            diff_files: Vec<FileMatch>,
            file: Option<CurrentFileMatch>,
            hunk: Option<CurrentHunk>,
            hunk_line: Option<BufferRef>,
            max_context: u32,
        }
        impl DiffParser {
            fn ensure_file(&mut self) -> &mut CurrentFileMatch {
                self.file.get_or_insert_with(|| CurrentFileMatch {
                    status_markers: Vec::from([MatchStatusMarker {
                        old_line: 0,
                        new_line: 0,
                        status: MatchStatus::Unchanged,
                    }]),
                    ..CurrentFileMatch::default()
                })
            }

            fn process_hunk_line(&mut self, buffer: &Buffer, lineref: BufferRef) -> Result<()> {
                if lineref.len() < 1 {
                    Err("completely empty hunk line")?;
                }

                let Some(file) = &mut self.file else { panic!() };
                let Some(hunk) = &mut self.hunk else { panic!() };

                if file.known_eof {
                    Err("hunk continues after end of file")?;
                }

                let ch = buffer[lineref][0];
                let (is_old, is_new) = match ch {
                    b' ' => (true, true),
                    b'-' => (true, false),
                    b'+' => (false, true),
                    _ => Err("unknown line start found inside hunk")?,
                };

                let status = if is_old && is_new {
                    hunk.unchanged_count += 1;
                    MatchStatus::Unchanged
                } else {
                    if !hunk.seen_changed {
                        self.max_context = std::cmp::max(self.max_context, hunk.unchanged_count);
                    }
                    hunk.unchanged_count = 0;
                    hunk.seen_changed = true;
                    MatchStatus::Changed { unimportant: false }
                };

                if file.status_markers.last().unwrap().status != status {
                    file.status_markers.push(MatchStatusMarker {
                        old_line: file.old_line,
                        new_line: file.new_line,
                        status,
                    });
                }

                if is_old {
                    let Some((_, old_file)) = file.old.as_mut() else {
                        return Err("line in hunk covers old file but there is no old file")?;
                    };

                    old_file.push_line(file.old_line, lineref.slice(1..), buffer)?;
                    file.old_line += 1;

                    if hunk.old_remaining == 0 {
                        Err("too many old lines in hunk")?;
                    }
                    hunk.old_remaining -= 1;
                }
                if is_new {
                    let Some((_, new_file)) = file.new.as_mut() else {
                        return Err("line in hunk covers new file but there is no new file")?;
                    };

                    new_file.push_line(file.new_line, lineref.slice(1..), buffer)?;
                    file.new_line += 1;

                    if hunk.new_remaining == 0 {
                        Err("too many new lines in hunk")?;
                    }
                    hunk.new_remaining -= 1;
                }

                if hunk.old_remaining == 0 && hunk.new_remaining == 0 {
                    if status != MatchStatus::Unchanged {
                        file.status_markers.push(MatchStatusMarker {
                            old_line: file.old_line,
                            new_line: file.new_line,
                            status: MatchStatus::Unchanged,
                        });
                    }
                    if file.known_eof_after_hunk {
                        file.known_eof = true;
                    }

                    self.max_context = std::cmp::max(self.max_context, hunk.unchanged_count);
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
            max_context: 0,
        };

        for (lineidx, lineref) in buffer
            .lines(range)
            .chain(std::iter::once(BufferRef::default()))
            .enumerate()
        {
            try_forward(
                || -> Result<()> {
                    let line = &buffer[lineref];

                    if let Some(mut hunk_line) = parser.hunk_line.take() {
                        let no_newline = line == b"\\ No newline at end of file";
                        if !no_newline {
                            if buffer.get(hunk_line.end) != Some(b'\n') {
                                Err("hunk line missing newline")?;
                            }
                            hunk_line.end += 1;
                        }

                        parser.process_hunk_line(&buffer, hunk_line)?;
                        if no_newline {
                            parser.file.as_mut().unwrap().known_eof = true;
                            return Ok(());
                        }
                    }

                    if parser.hunk.is_some() {
                        parser.hunk_line = Some(lineref);
                        return Ok(());
                    }

                    if line.starts_with(b"@@ ") {
                        let Some(file) = &mut parser.file else {
                            return Err("hunk without file")?;
                        };

                        if file.known_eof {
                            Err("hunk after known EOF for current file")?;
                        }

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

                        if old_start == 0 {
                            if old_count != 0 {
                                Err("surprising old line reference")?;
                            }
                            old_start = 1;
                            file.known_eof_after_hunk = true;
                        }
                        if new_start == 0 {
                            if new_count != 0 {
                                Err("surprising new line reference")?;
                            }
                            new_start = 1;
                            file.known_eof_after_hunk = true;
                        }

                        old_start -= 1;
                        new_start -= 1;

                        if old_start < file.old_line || new_start < file.new_line {
                            Err("hunks seem to be out of order or otherwise inconsistent?")?;
                        }

                        file.old_line = old_start;
                        file.new_line = new_start;

                        parser.hunk = Some(CurrentHunk {
                            old_remaining: old_count,
                            new_remaining: new_count,
                            unchanged_count: 0,
                            seen_changed: false,
                        });

                        return Ok(());
                    }

                    if let Some(file) = parser.file.take() {
                        if file.new_path.is_some() {
                            let mut file_match = FileMatch {
                                old_path: buffer[file.old_path.unwrap()].to_vec(),
                                new_path: buffer[file.new_path.unwrap()].to_vec(),
                                old: file
                                    .old
                                    .map(|(name, b)| b.build(name, file.known_eof, &buffer)),
                                new: file
                                    .new
                                    .map(|(name, b)| b.build(name, file.known_eof, &buffer)),
                                status_markers: file.status_markers,
                            };
                            file_match.simplify();
                            parser.diff_files.push(file_match);
                        } else {
                            parser.file = Some(file);
                        }
                    }

                    if line.starts_with(b"--- ") {
                        let file = parser.ensure_file();
                        if file.old_path.is_some() {
                            Err("multiple '---' lines found")?;
                        }

                        file.old_path = Some(lineref.slice(4..));
                        let name_ref = parse_diff_path(
                            lineref.slice(4..),
                            diff_options.strip_path_components,
                            &buffer,
                        )?;
                        if let Some(name_ref) = name_ref {
                            file.old = Some((name_ref, FileBuilder::new()));
                        }
                        return Ok(());
                    }
                    if line.starts_with(b"+++ ") {
                        let file = parser.ensure_file();
                        if file.old_path.is_none() {
                            Err("found '+++' line without preceding '---' line")?;
                        }
                        if file.new_path.is_some() {
                            Err("multiple '+++' lines found")?;
                        }

                        file.new_path = Some(lineref.slice(4..));
                        let name_ref = parse_diff_path(
                            lineref.slice(4..),
                            diff_options.strip_path_components,
                            &buffer,
                        )?;
                        if let Some(name_ref) = name_ref {
                            file.new = Some((name_ref, FileBuilder::new()));
                        }
                        return Ok(());
                    }

                    if parser.file.is_some() {
                        Err("unrecognized noise in file")?;
                    }

                    // Just skip noise outside of a file region.
                    Ok(())
                },
                move || format!("line {}", lineidx + 1),
            )?;
        }

        if parser.hunk_line.is_some() {
            Err("incomplete hunk at end of diff")?;
        }
        assert!(parser.file.is_none());

        diff_options.num_context_lines = parser.max_context as usize;

        Ok(Diff {
            files: parser.diff_files,
            options: diff_options,
        })
    }

    pub fn render(&self, buffer: &Buffer, writer: &mut dyn render::ChunkWriter) {
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
        let mut writer = render::ChunkByteBufferWriter::new();
        self.diff.render(self.buffer, &mut writer);
        write!(f, "{}", String::from_utf8_lossy(&writer.out))
    }
}

/// An index over a diff file that allows quick lookup of files and hunks.
///
/// TODO: Actually implement this as an index?
#[derive(Debug)]
pub struct DiffIndex<'a> {
    buffer: &'a Buffer,
    diff: &'a Diff,
}

impl<'a> DiffIndex<'a> {
    pub fn create(diff: &'a Diff, buffer: &'a Buffer) -> Self {
        Self { buffer, diff }
    }

    pub fn find_old_file_by_name<'b, 'c>(&'b self, name: &'c [u8]) -> Option<&'b FileMatch> {
        self.diff.files.iter().find(|file| {
            file.old
                .as_ref()
                .is_some_and(|f| *f.name(self.buffer) == *name)
        })
    }

    pub fn find_old_file_by_name_ref<'b>(&'b self, name: BufferRef) -> Option<&'b FileMatch> {
        self.find_old_file_by_name(&self.buffer[name])
    }

    pub fn find_new_file_by_name<'b, 'c>(&'b self, name: &'c [u8]) -> Option<&'b FileMatch> {
        self.diff.files.iter().find(|file| {
            file.new
                .as_ref()
                .is_some_and(|f| *f.name(self.buffer) == *name)
        })
    }

    pub fn find_new_file_by_name_ref<'b>(&'b self, name: BufferRef) -> Option<&'b FileMatch> {
        self.find_new_file_by_name(&self.buffer[name])
    }
}

/// Compute the diff that results from composing `first` followed by `second`.
///
/// Note: This function performs a trivial simplification of the diff but does
/// not look for opportunities for further simplification in case the second
/// diff (partially) reverts the first one.
pub fn compose(first: &Diff, second: &Diff, buffer: &Buffer) -> Result<Diff> {
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

    let first_diff_idx = DiffIndex::create(first, buffer);
    let second_diff_idx = DiffIndex::create(second, buffer);

    let mut recreated: HashSet<&[u8]> = HashSet::new();

    for first_file_match in &first.files {
        if let Some(second_file_match) = first_file_match
            .new
            .as_ref()
            .and_then(|new| second_diff_idx.find_old_file_by_name_ref(new.name_ref()))
        {
            let mut status_markers = Vec::new();
            let mut old_file = FileBuilder::new();
            let mut new_file = FileBuilder::new();

            // Conceptually walk along the lines of the middle file and use them as a "zipper"
            // that trigger forward movement over the status markers in the first and second match.
            //
            // Fill in lines on both sides as we go along, leveraging unchanged statuses to fill in
            // lines that are only known via one of the input matches.
            assert!(first_file_match.status_markers[0].old_line == 0);
            assert!(first_file_match.status_markers[0].new_line == 0);
            assert!(second_file_match.status_markers[0].old_line == 0);
            assert!(second_file_match.status_markers[0].new_line == 0);
            let mut first_idx = 1;
            let mut second_idx = 1;
            let mut old_line = 0;
            let mut mid_line = 0;
            let mut new_line = 0;

            while first_idx < first_file_match.status_markers.len()
                || second_idx < second_file_match.status_markers.len()
            {
                let first_status = first_file_match.status_markers[first_idx - 1].status;
                let second_status = second_file_match.status_markers[second_idx - 1].status;

                status_markers.push(MatchStatusMarker {
                    old_line,
                    new_line,
                    status: first_status.merge(second_status),
                });

                let (old_end, mid_end, new_end) = {
                    let mut first_sm = first_file_match.status_markers.get(first_idx);
                    let mut second_sm = second_file_match.status_markers.get(second_idx);
                    if let (Some(the_first_sm), Some(the_second_sm)) = (first_sm, second_sm) {
                        if the_first_sm.new_line <= the_second_sm.old_line {
                            second_sm = None;
                        } else {
                            first_sm = None;
                        }
                    }

                    match (first_sm, second_sm) {
                        (Some(first_sm), None) => {
                            first_idx += 1;
                            let new_end = if second_status.is_changed() {
                                new_line
                            } else {
                                new_line + (first_sm.new_line - mid_line)
                            };
                            (first_sm.old_line, first_sm.new_line, new_end)
                        }
                        (None, Some(second_sm)) => {
                            second_idx += 1;
                            let old_end = if first_status.is_changed() {
                                old_line
                            } else {
                                old_line + (second_sm.old_line - mid_line)
                            };
                            (old_end, second_sm.old_line, second_sm.new_line)
                        }
                        _ => unreachable!(),
                    }
                };

                let mut old_line_cur = old_line;
                while old_line_cur != old_end {
                    let remaining_first;
                    (old_line_cur, remaining_first) = old_file.copy_known_lines(
                        old_line_cur,
                        first_file_match.old.as_ref().unwrap(),
                        old_line_cur..old_end,
                        buffer,
                    )?;
                    if first_status == MatchStatus::Unchanged {
                        let mut gap = mid_line + (old_line_cur - old_line)
                            ..mid_line + (remaining_first.start - old_line);
                        while !gap.is_empty() {
                            (_, gap) = old_file.copy_known_lines(
                                old_line_cur,
                                second_file_match.old.as_ref().unwrap(),
                                gap,
                                buffer,
                            )?;
                            old_line_cur = old_line + (gap.start - mid_line);
                        }
                    }
                    old_line_cur = remaining_first.start;
                }

                let mut new_line_cur = new_line;
                while new_line_cur != new_end {
                    let remaining_second;
                    (new_line_cur, remaining_second) = new_file.copy_known_lines(
                        new_line_cur,
                        second_file_match.new.as_ref().unwrap(),
                        new_line_cur..new_end,
                        buffer,
                    )?;
                    if second_status == MatchStatus::Unchanged {
                        let mut gap = mid_line + (new_line_cur - new_line)
                            ..mid_line + (remaining_second.start - new_line);
                        while !gap.is_empty() {
                            (_, gap) = new_file.copy_known_lines(
                                new_line_cur,
                                first_file_match.new.as_ref().unwrap(),
                                gap,
                                buffer,
                            )?;
                            new_line_cur = new_line + (gap.start - mid_line);
                        }
                    }
                    new_line_cur = remaining_second.start;
                }

                old_line = old_end;
                mid_line = mid_end;
                new_line = new_end;
            }

            status_markers.push(MatchStatusMarker {
                old_line,
                new_line,
                status: MatchStatus::Unchanged,
            });

            // Fill in unchanged lines at end of file.
            if let (Some(old_orig), Some(new_orig)) =
                (&first_file_match.old, &second_file_match.new)
            {
                let mut outer_gap = old_line..u32::MAX;
                while !outer_gap.is_empty() {
                    let outer_gap_next;
                    (old_line, outer_gap_next) =
                        old_file.copy_known_lines(old_line, old_orig, outer_gap.clone(), buffer)?;
                    (new_line, _) =
                        new_file.copy_known_lines(new_line, old_orig, outer_gap.clone(), buffer)?;

                    let mut inner_gap =
                        new_line..new_line.saturating_add(outer_gap_next.start - old_line);
                    while !inner_gap.is_empty() {
                        let inner_gap_next;
                        (old_line, inner_gap_next) = old_file.copy_known_lines(
                            old_line,
                            new_orig,
                            inner_gap.clone(),
                            buffer,
                        )?;
                        (new_line, _) = new_file.copy_known_lines(
                            new_line,
                            new_orig,
                            inner_gap.clone(),
                            buffer,
                        )?;
                        inner_gap = inner_gap_next;
                    }

                    outer_gap = outer_gap_next;
                }
            }

            let old_orig = first_file_match.old.as_ref();
            let new_orig = second_file_match.new.as_ref();
            result.files.push(FileMatch {
                old_path: first_file_match.old_path.clone(),
                old: old_orig.map(|old_orig| {
                    old_file.build(old_orig.name_ref(), old_orig.num_lines().is_some(), buffer)
                }),
                new_path: second_file_match.new_path.clone(),
                new: new_orig.map(|new_orig| {
                    new_file.build(new_orig.name_ref(), new_orig.num_lines().is_some(), buffer)
                }),
                status_markers,
            });
            continue;
        }

        // Find "recreated" files.
        //
        // TODO: Ideally, this would be able to track renames to some extent.
        if first_file_match.new.is_none() {
            if let Some(second_file_match) = second_diff_idx
                .find_new_file_by_name_ref(first_file_match.old.as_ref().unwrap().name_ref())
            {
                if second_file_match.old.is_none() {
                    assert!(first_file_match.status_markers.len() == 2);
                    assert!(second_file_match.status_markers.len() == 2);

                    let status = first_file_match.status_markers[0]
                        .status
                        .merge(second_file_match.status_markers[0].status);
                    let status_markers = vec![
                        MatchStatusMarker {
                            old_line: 0,
                            new_line: 0,
                            status,
                        },
                        MatchStatusMarker {
                            old_line: first_file_match.status_markers[1].old_line,
                            new_line: second_file_match.status_markers[1].new_line,
                            status: MatchStatus::Unchanged,
                        },
                    ];

                    result.files.push(FileMatch {
                        old_path: first_file_match.old_path.clone(),
                        old: first_file_match.old.clone(),
                        new_path: second_file_match.new_path.clone(),
                        new: second_file_match.new.clone(),
                        status_markers,
                    });
                    recreated.insert(second_file_match.new.as_ref().unwrap().name(buffer));
                    continue;
                }
            }
        }

        result.files.push(first_file_match.clone());
    }

    for second_file_match in &second.files {
        if second_file_match.old.is_none()
            && recreated.contains(&second_file_match.new.as_ref().unwrap().name(buffer))
        {
            continue;
        }
        if second_file_match.old.as_ref().is_none_or(|old| {
            first_diff_idx
                .find_new_file_by_name_ref(old.name_ref())
                .is_none()
        }) {
            result.files.push(second_file_match.clone());
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
        std::mem::swap(&mut file.old, &mut file.new);

        for sm in &mut file.status_markers {
            std::mem::swap(&mut sm.old_line, &mut sm.new_line);
        }
    }

    result
}

/// Reduce the `target` diff based on knowledge about the `old` and `new` diffs.
fn reduce_modulo_base(
    mut target: Diff,
    target_is_base: bool,
    base_old: &Diff,
    base_new: &Diff,
    buffer: &Buffer,
) -> Result<Diff> {
    let base_old_index = DiffIndex::create(base_old, buffer);
    let base_new_index = DiffIndex::create(base_new, buffer);

    target.files = target
        .files
        .into_iter()
        .filter_map(|mut target| {
            let base_old;
            let base_new;
            if target_is_base {
                base_old = target
                    .old
                    .as_ref()
                    .and_then(|file| base_old_index.find_old_file_by_name_ref(file.name_ref()));
                base_new = target
                    .new
                    .as_ref()
                    .and_then(|file| base_new_index.find_old_file_by_name_ref(file.name_ref()));
            } else {
                base_old = target
                    .old
                    .as_ref()
                    .and_then(|file| base_old_index.find_new_file_by_name_ref(file.name_ref()));
                base_new = target
                    .new
                    .as_ref()
                    .and_then(|file| base_new_index.find_new_file_by_name_ref(file.name_ref()));
            }

            if base_old.is_none() && base_new.is_none() {
                // The file is affected by neither the base..old nor the base..new
                // diff. We should remove it entirely.
                return None;
            }

            for sm_idx in 0..target.status_markers.len() - 1 {
                let (sm, rest) = target
                    .status_markers
                    .get_mut(sm_idx..)
                    .unwrap()
                    .split_first_mut()
                    .unwrap();
                let MatchStatus::Changed { unimportant } = &mut sm.status else {
                    continue;
                };

                let sm_next = &rest[0];

                // If the lines touched by this change in the target diff are unchanged by the
                // base..old and base..new diffs, the change is unimportant.
                let unchanged = base_old.is_some_and(|base_old| {
                    base_old.lines_unchanged_halo(!target_is_base, sm.old_line..sm_next.old_line)
                }) && base_new.is_some_and(|base_new| {
                    base_new.lines_unchanged_halo(!target_is_base, sm.new_line..sm_next.new_line)
                });

                if unchanged {
                    *unimportant = true;
                }
            }

            Some(target)
        })
        .collect();

    Ok(target)
}

pub fn diff_modulo_base(
    buffer: &Buffer,
    target: Diff,
    base_old: &Diff,
    base_new: &Diff,
    writer: &mut dyn render::ChunkWriter,
) -> Result<()> {
    let base = compose(base_old, &target, buffer)?;
    let base = compose(&base, &reverse(base_new), buffer)?;
    let base = reduce_modulo_base(base, true, base_old, base_new, buffer)?;
    let base = reduce_changed_diff(buffer, base, DiffAlgorithm::default());

    let target = reduce_modulo_base(target, false, base_old, base_new, buffer)?;

    let base_old_index = DiffIndex::create(base_old, buffer);
    let base_new_index = DiffIndex::create(base_new, buffer);
    let base_index = DiffIndex::create(&base, buffer);

    let num_context_lines = std::cmp::max(
        base.options.num_context_lines,
        target.options.num_context_lines,
    );

    for target_file in &target.files {
        let base_file = target_file
            .old
            .as_ref()
            .and_then(|old| base_old_index.find_new_file_by_name_ref(old.name_ref()))
            .and_then(|base_old_file| base_old_file.old.as_ref())
            .and_then(|old_old| base_index.find_old_file_by_name_ref(old_old.name_ref()))
            .or_else(|| {
                target_file
                    .new
                    .as_ref()
                    .and_then(|new| base_new_index.find_new_file_by_name_ref(new.name_ref()))
                    .and_then(|base_new_file| base_new_file.old.as_ref())
                    .and_then(|new_old| base_index.find_new_file_by_name_ref(new_old.name_ref()))
            });

        if let Some(base_file) = base_file {
            let mut need_base_header = false;
            let mut need_target_header = false;

            let mut base_hunks = hunkify(base_file, Some(num_context_lines), buffer).peekable();
            let mut target_hunks = hunkify(target_file, Some(num_context_lines), buffer).peekable();

            let mut hunks: Vec<(render::Context, Hunk)> = Vec::new();

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
                    hunks.push((render::Context::Baseline, base_hunks.next().unwrap()));
                    need_base_header = true;
                } else {
                    hunks.push((render::Context::Change, target_hunks.next().unwrap()));
                    need_target_header = true;
                }
            }

            if need_base_header {
                base_file
                    .render_header(buffer, &mut writer.with_context(render::Context::Baseline));
            }
            if need_target_header {
                target_file
                    .render_header(buffer, &mut writer.with_context(render::Context::Change));
            }

            for (context, hunk) in hunks {
                hunk.render(true, &mut writer.with_context(context));
            }
        } else {
            target_file.render(
                buffer,
                num_context_lines,
                &mut writer.with_context(render::Context::Change),
            );
        }
    }

    Ok(())
}

pub fn diff_file(
    buffer: &Buffer,
    old_path: BufferRef,
    new_path: BufferRef,
    old_body: BufferRef,
    new_body: BufferRef,
    options: &DiffOptions,
    algorithm: DiffAlgorithm,
) -> Result<FileMatch> {
    let old_name = parse_diff_path(old_path, options.strip_path_components, buffer)?;
    if old_name.is_none() && old_body.len() > 0 {
        let old_path = String::from_utf8_lossy(&buffer[old_path]);
        Err(format!(
            "have non-empty old body but old path '{}' suggests missing file",
            old_path
        ))?;
    }
    let old = old_name.map(|name_ref| {
        let mut fb = FileBuilder::new();
        fb.push_text(0, old_body, buffer).unwrap();
        fb.build(name_ref, true, buffer)
    });

    let new_name = parse_diff_path(new_path, options.strip_path_components, buffer)?;
    if new_name.is_none() && new_body.len() > 0 {
        let new_path = String::from_utf8_lossy(&buffer[new_path]);
        Err(format!(
            "have non-empty new body but new path '{}' suggests missing file",
            new_path
        ))?;
    }
    let new = new_name.map(|name_ref| {
        let mut fb = FileBuilder::new();
        fb.push_text(0, new_body, buffer).unwrap();
        fb.build(name_ref, true, buffer)
    });

    let old_lines = old.as_ref().and_then(|f| f.num_lines()).unwrap_or(0);
    let new_lines = new.as_ref().and_then(|f| f.num_lines()).unwrap_or(0);

    let file = FileMatch {
        old_path: buffer[old_path].to_owned(),
        old,
        new_path: buffer[new_path].to_owned(),
        new,
        status_markers: vec![
            MatchStatusMarker {
                old_line: 0,
                new_line: 0,
                status: MatchStatus::Changed { unimportant: false },
            },
            MatchStatusMarker {
                old_line: old_lines,
                new_line: new_lines,
                status: MatchStatus::Unchanged,
            },
        ],
    };

    Ok(reduce_changed_file(buffer, file, algorithm).0)
}
