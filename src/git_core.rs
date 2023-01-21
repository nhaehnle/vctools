// SPDX-License-Identifier: MIT

use std::{io::prelude::*, fmt::Display};

use crate::utils::{Result, err_from_str, try_forward, trim_ascii};

pub use std::ops::Range;
use lazy_static::lazy_static;
use regex::bytes::Regex;

/// Reference to a single commit, using any format the git CLI understands as
/// a reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    name: String,
}
impl Ref {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
        }
    }

    pub fn first_parent(&self) -> Self {
        Self::new(format!("{}^", self.name))
    }
}
impl Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.name, f)
    }
}

#[derive(Debug, Clone)]
pub struct ShowOptions {
    pub show_patch: bool,
}
impl Default for ShowOptions {
    fn default() -> Self {
        Self {
            show_patch: true,
        }
    }
}

#[derive(Debug)]
pub struct Repository {
    pub path: std::path::PathBuf,
}
impl Repository {
    pub fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.into(),
        }
    }

    fn exec<'a, I, A>(&self, subcommand: &str, args: I) -> Result<Vec<u8>>
        where I: Iterator<Item = A>,
              A: AsRef<std::ffi::OsStr>
    {
        let mut cmd = std::process::Command::new("git");
        cmd.args(["-C", self.path.to_str().unwrap()]);
        cmd.arg(subcommand);
        cmd.args(args);

        // We use a somewhat complex dance for reading stdout and stderr in
        // parallel, because the default behavior of Command::output() on Linux
        // uses a poll(2) loop that reads one pipe buffer's worth of data at a
        // time, which is terribly CPU inefficient and kills parallelism between
        // us and the child process. The end result is we can't keep up with
        // git's output and slow everything down terribly if git output (e.g.
        // a diff) happens to be large (on one system, using plain
        // Command::output lead to a ~9x slowdown).
        //
        // Using the extra thread feels painful, but it seems the best portable
        // solution that avoids pulling in a large dependency.
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        let mut stderr = child.stderr.take().unwrap();
        let stderr_thread = std::thread::spawn(move || -> std::result::Result<Vec<u8>, String> {
            let mut stderr_buf = Vec::new();
            match stderr.read_to_end(&mut stderr_buf) {
            Ok(_) => Ok(stderr_buf),
            Err(e) => Err(format!("reading stderr: {e}")),
            }
        });
        let output = child.wait_with_output()?;
        let stderr = stderr_thread.join().unwrap()?;

        if !output.status.success() || !stderr.is_empty() {
            return Err(format!("git subcommand failed: {}\n{}", output.status,
                               String::from_utf8_lossy(&stderr)).into())
        }

        Ok(output.stdout)
    }

    pub fn diff(&self, range: Range<&Ref>, paths: Option<&[&[u8]]>) -> Result<Vec<u8>> {
        try_forward(|| -> Result<Vec<u8>> {
            let mut args: Vec<String> = Vec::new();
            args.push(format!("{}..{}", range.start, range.end));
            if let Some(paths) = paths {
                args.push("--".into());
                args.extend(paths.iter().map(|&s| String::from_utf8_lossy(s).into()));
            }

            self.exec("diff", args.iter())
        }, || format!("failed to get diff {}..{}", range.start, range.end))
    }

    pub fn diff_commit(&self, commit: &Ref, paths: Option<&[&[u8]]>) -> Result<Vec<u8>> {
        self.diff(&commit.first_parent()..commit, paths)
    }

    pub fn show_commit(&self, commit: &Ref, options: &ShowOptions) -> Result<Vec<u8>> {
        try_forward(|| -> Result<Vec<u8>> {
            let mut args: Vec<String> = Vec::new();
            if !options.show_patch {
                args.push("--no-patch".into());
            }
            args.push(format!("{}", commit));

            self.exec("show", args.iter())
        }, || format!("failed to show {}", commit))
    }

    pub fn merge_base(&self, a: &Ref, b: &Ref) -> Result<Ref> {
        try_forward(|| -> Result<Ref> {
            let result = self.exec("merge-base", [
                format!("{a}"),
                format!("{b}"),
            ].iter())?;

            Ok(Ref::new(String::from_utf8_lossy(trim_ascii(&result))))
        }, || "failed to obtain merge-base")
    }

    pub fn range_diff<R>(&self, old: Range<R>, new: Range<R>) -> Result<RangeDiff>
        where R: std::borrow::Borrow<Ref>,
    {
        try_forward(|| -> Result<RangeDiff> {
            let result = self.exec("range-diff", [
                "-s".into(),
                format!("{}..{}", old.start.borrow(), old.end.borrow()),
                format!("{}..{}", new.start.borrow(), new.end.borrow()),
            ].iter())?;

            RangeDiff::parse(&result)
        }, || "failed to obtain range-diff")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeDiffMatch {
    pub changed: bool,
    pub old: Option<(u32, Ref)>,
    pub new: Option<(u32, Ref)>,
    pub title: Vec<u8>,
}

#[derive(Debug)]
pub struct RangeDiff {
    pub matches: Vec<RangeDiffMatch>,
}
impl RangeDiff {
    fn parse_impl(buffer: &[u8]) -> Result<Self> {
        lazy_static! {
            static ref RE: Regex = Regex::new(
                r"(?-u)(?:(-+)|([0-9]+)): +(?:(-+)|([0-9a-f]+)) ([!<>=]) (?:(-+)|([0-9]+)): +(?:(-+)|([0-9a-f]+)) +(.*)"
            ).unwrap();
        }

        let mut matches = Vec::new();
        for line in buffer.split(|&ch| ch == b'\n') {
            let line = trim_ascii(line);
            if line.is_empty() {
                continue
            }

            let captures = RE.captures(line).ok_or_else(|| err_from_str("bad diff-range line"))?;

            fn get_side(captures: &regex::bytes::Captures, idx: usize) -> Result<Option<(u32, Ref)>> {
                let index_missing = captures.get(idx);
                let index_number = captures.get(idx + 1);
                let commit_missing = captures.get(idx + 2);
                let commit_hash = captures.get(idx + 3);

                if index_missing.is_some() != commit_missing.is_some() {
                    return Err("one of index and commit missing, but not both".into())
                }

                if let (Some(index_number), Some(commit_hash)) = (index_number, commit_hash) {
                    let index: u32 = std::str::from_utf8(index_number.as_bytes())?.parse()?;
                    let commit = Ref::new(std::str::from_utf8(commit_hash.as_bytes())?);
                    Ok(Some((index, commit)))
                } else {
                    Ok(None)
                }
            }

            let old = try_forward(|| get_side(&captures, 1), || "left hand side")?;
            let new = try_forward(|| get_side(&captures, 6), || "left hand side")?;

            let (changed, old_expected, new_expected) =
                match captures.get(5).unwrap().as_bytes() {
                b"=" => (false, true, true),
                b"!" => (true, true, true),
                b">" => (true, false, true),
                b"<" => (true, true, false),
                other => return Err(format!("bad change indicator '{}'", String::from_utf8_lossy(other)).into()),
                };

            if old.is_some() != old_expected || new.is_some() != new_expected {
                return Err("change indicator doesn't match the shown commits".into())
            }

            matches.push(RangeDiffMatch {
                changed,
                old,
                new,
                title: captures.get(10).unwrap().as_bytes().into(),
            });
        }

        Ok(RangeDiff { matches })
    }

    fn parse(buffer: &[u8]) -> Result<Self> {
        try_forward(|| Self::parse_impl(buffer), || "parsing range-diff")
    }
}

#[cfg(test)]
mod test {
    use crate::git_core::*;

    #[test]
    fn range_diff_basic() -> Result<()> {
        let range_diff_text = "\
            1:  31b5c003 ! 1:  d73727e2 title foo\n\
            -:  -------- > 2:  98ad5553 title blah\n\
            3:  01234567 < -:  -------- blub\n\
            2:  89abcdef = 3:  fedc3210 another\n\
        ";

        let rd = RangeDiff::parse(range_diff_text.as_bytes())?;

        assert_eq!(rd.matches.len(), 4);
        assert_eq!(rd.matches[0], RangeDiffMatch {
            changed: true,
            old: Some((1, Ref::new("31b5c003"))),
            new: Some((1, Ref::new("d73727e2"))),
            title: (*b"title foo").into(),
        });
        assert_eq!(rd.matches[1], RangeDiffMatch {
            changed: true,
            old: None,
            new: Some((2, Ref::new("98ad5553"))),
            title: (*b"title blah").into(),
        });
        assert_eq!(rd.matches[2], RangeDiffMatch {
            changed: true,
            old: Some((3, Ref::new("01234567"))),
            new: None,
            title: (*b"blub").into(),
        });
        assert_eq!(rd.matches[3], RangeDiffMatch {
            changed: false,
            old: Some((2, Ref::new("89abcdef"))),
            new: Some((3, Ref::new("fedc3210"))),
            title: (*b"another").into(),
        });

        Ok(())
    }
}
