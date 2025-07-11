// SPDX-License-Identifier: MIT

use std::{fmt::Display, io::prelude::*};

use crate::utils::{trim_ascii, try_forward, Result};

use lazy_static::lazy_static;
use regex::bytes::Regex;
pub use std::ops::Range;

/// Reference to a single commit, using any format the git CLI understands as
/// a reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    pub name: String,
}
impl Ref {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self { name: name.into() }
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
    pub skip_commit_id: bool,
}
impl Default for ShowOptions {
    fn default() -> Self {
        Self {
            show_patch: true,
            skip_commit_id: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Url {
    Ssh {
        user: Option<String>,
        host: String,
        path: String,
    },
    Url(reqwest::Url),
}
impl Url {
    pub fn hostname(&self) -> Option<&str> {
        match self {
            Url::Ssh { host, .. } => Some(&host),
            Url::Url(url) => url.host_str(),
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Url::Ssh { path, .. } => &path,
            Url::Url(url) => url.path().strip_prefix("/").unwrap_or_default(),
        }
    }

    // Returns (organization, repository) from a GitHub URL.
    pub fn github_path(&self) -> Option<(&str, &str)> {
        let path = self.path();
        let path = path.strip_suffix(".git").unwrap_or(path);
        let mut iter = path.split("/");
        let organization = iter.next()?;
        let repo = iter.next()?;
        if iter.next().is_some() {
            None
        } else {
            Some((organization, repo))
        }
    }
}
impl Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Url::Ssh { user, host, path } => {
                if let Some(user) = user {
                    write!(f, "{}@", user)?;
                }
                write!(f, "{}:{}", host, path)
            }
            Url::Url(url) => write!(f, "{}", url),
        }
    }
}

#[derive(Debug)]
pub struct Repository {
    pub path: std::path::PathBuf,

    // Path of a directory that contains mock outputs of git commits as plain text files named with
    // the command line after the "git" command itself. For example, a file named "show main" would
    // contain the output of "git show main".
    pub mock_data_path: Option<std::path::PathBuf>,
}
impl Repository {
    pub fn new(path: &std::path::Path) -> Self {
        Self {
            path: path.into(),
            mock_data_path: None,
        }
    }

    fn exec_with_stderr<'a, I, A>(&self, subcommand: &str, args: I) -> Result<(Vec<u8>, Vec<u8>)>
    where
        I: Iterator<Item = A>,
        A: AsRef<std::ffi::OsStr>,
    {
        if let Some(test_data_path) = &self.mock_data_path {
            let mut path = test_data_path.clone();
            let components: Vec<_> = [std::ffi::OsString::from(subcommand)]
                .into_iter()
                .chain(args.map(|x| x.as_ref().to_os_string()))
                .collect();
            let cmdline = components.join(&std::ffi::OsString::from(" "));
            let mut name = cmdline.to_string_lossy().to_string();
            name.retain(|c| c != '/');
            path.push(&name);

            let mut file = try_forward(
                || Ok(std::fs::File::open(&path)?),
                || {
                    format!(
                        "failed to open mock data file {} for `git {}`",
                        &name,
                        cmdline.to_string_lossy()
                    )
                },
            )?;
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;

            return Ok((contents, Vec::new()));
        }

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

        if !output.status.success() {
            return Err(format!(
                "git subcommand failed: {}\n{}",
                output.status,
                String::from_utf8_lossy(&stderr)
            )
            .into());
        }

        Ok((output.stdout, output.stderr))
    }

    fn exec<'a, I, A>(&self, subcommand: &str, args: I) -> Result<Vec<u8>>
    where
        I: Iterator<Item = A>,
        A: AsRef<std::ffi::OsStr>,
    {
        let (stdout, stderr) = self.exec_with_stderr(subcommand, args)?;

        if !stderr.is_empty() {
            Err(format!(
                "git subcommand produced unexpected stderr: {}",
                String::from_utf8_lossy(&stderr),
            ))?;
        }

        Ok(stdout)
    }

    pub fn get_url(&self, remote: &str) -> Result<Url> {
        try_forward(
            || -> Result<Url> {
                let raw = self.exec("remote", [&"get-url", remote].iter())?;
                let url = String::from_utf8(raw)?;
                let url = url.trim();

                lazy_static! {
                    static ref GIT_RE: regex::Regex =
                        regex::Regex::new(r"^(?:([^@/:]+)@)?([^@/:]+):([^@:]+)$").unwrap();
                }

                if let Some(captures) = GIT_RE.captures(&url) {
                    let host = captures.get(2).unwrap().as_str();
                    let path = captures.get(3).unwrap().as_str();

                    return Ok(Url::Ssh {
                        user: captures.get(1).map(|x| x.as_str().into()),
                        host: host.into(),
                        path: path.into(),
                    });
                }

                Ok(Url::Url(reqwest::Url::parse(&url)?))
            },
            || format!("failed to query URL for remote {}", remote),
        )
    }

    pub fn diff(&self, range: Range<&Ref>, paths: Option<&[&[u8]]>) -> Result<Vec<u8>> {
        try_forward(
            || -> Result<Vec<u8>> {
                let mut args: Vec<String> = Vec::new();
                args.push(format!("{}..{}", range.start, range.end));
                if let Some(paths) = paths {
                    args.push("--".into());
                    args.extend(paths.iter().map(|&s| String::from_utf8_lossy(s).into()));
                }

                self.exec("diff", args.iter())
            },
            || format!("failed to get diff {}..{}", range.start, range.end),
        )
    }

    pub fn diff_commit(&self, commit: &Ref, paths: Option<&[&[u8]]>) -> Result<Vec<u8>> {
        self.diff(&commit.first_parent()..commit, paths)
    }

    pub fn show_commit(&self, commit: &Ref, options: &ShowOptions) -> Result<Vec<u8>> {
        try_forward(
            || -> Result<Vec<u8>> {
                let mut args: Vec<String> = Vec::new();
                if !options.show_patch {
                    args.push("--no-patch".into());
                }
                args.push(format!("{}", commit));

                let mut show = self.exec("show", args.iter())?;

                // Erase the first line if it is of the form "commit <...>"
                if options.skip_commit_id && show.starts_with(b"commit ") {
                    if let Some(pos) = show.iter().position(|ch| *ch == b'\n') {
                        show = Vec::from(show.split_at(pos + 1).1);
                    }
                }

                Ok(show)
            },
            || format!("failed to show {}", commit),
        )
    }

    pub fn merge_base(&self, a: &Ref, b: &Ref) -> Result<Ref> {
        try_forward(
            || -> Result<Ref> {
                let result = self.exec("merge-base", [format!("{a}"), format!("{b}")].iter())?;

                Ok(Ref::new(String::from_utf8_lossy(trim_ascii(&result))))
            },
            || "failed to obtain merge-base",
        )
    }

    pub fn rev_parse(&self, a: &Ref) -> Result<Ref> {
        try_forward(
            || -> Result<Ref> {
                let result = self.exec("rev-parse", [format!("{a}")].iter())?;

                Ok(Ref::new(String::from_utf8_lossy(trim_ascii(&result))))
            },
            || "failed to obtain parsed revision",
        )
    }

    pub fn fetch_missing(&self, remote: &str, refs: &[Ref]) -> Result<()> {
        try_forward(
            || -> Result<()> {
                // Test if the refs are present
                if self
                    .exec(
                        "show",
                        ["--oneline"]
                            .into_iter()
                            .chain(refs.iter().map(|r| r.name.as_str())),
                    )
                    .is_ok()
                {
                    return Ok(());
                }

                // At least one failed, try to fetch them
                self.exec_with_stderr(
                    "fetch",
                    [remote]
                        .into_iter()
                        .chain(refs.iter().map(|r| r.name.as_str())),
                )?;

                Ok(())
            },
            || "failed to fetch missing refs",
        )
    }

    pub fn log<R>(&self, range: Range<R>) -> Result<Vec<LogEntry>>
    where
        R: std::borrow::Borrow<Ref>,
    {
        try_forward(
            || -> Result<Vec<LogEntry>> {
                let result = self.exec(
                    "log",
                    [
                        "--oneline".into(),
                        format!("{}..{}", range.start.borrow(), range.end.borrow()),
                    ]
                    .iter(),
                )?;

                lazy_static! {
                    static ref RE: Regex = Regex::new(r"([0-9a-f]+) +(.*)").unwrap();
                }

                let mut entries = Vec::new();

                for line in result.split(|&ch| ch == b'\n') {
                    let line = trim_ascii(line);
                    if line.is_empty() {
                        continue;
                    }

                    let captures = RE.captures(line).ok_or_else(|| {
                        format!("bad log line\n{}", String::from_utf8_lossy(line))
                    })?;

                    let commit = captures.get(1).unwrap().as_bytes();
                    let title = captures.get(2).unwrap().as_bytes();

                    entries.push(LogEntry {
                        commit: Ref::new(String::from_utf8(commit.into())?),
                        title: title.into(),
                    });
                }

                Ok(entries)
            },
            || "failed to obtain log",
        )
    }

    pub fn range_diff<R>(&self, old: Range<R>, new: Range<R>) -> Result<RangeDiff>
    where
        R: std::borrow::Borrow<Ref>,
    {
        // Workaround: git range-diff fails if start and end of a range are the same
        if old.start.borrow() == old.end.borrow() {
            let new_commits = self.log(new)?;
            return Ok(RangeDiff {
                matches: new_commits
                    .into_iter()
                    .enumerate()
                    .map(|(idx, entry)| RangeDiffMatch {
                        changed: true,
                        old: None,
                        new: Some((idx as u32 + 1, entry.commit)),
                        title: entry.title,
                    })
                    .collect(),
            });
        }

        if new.start.borrow() == new.end.borrow() {
            let old_commits = self.log(old)?;
            return Ok(RangeDiff {
                matches: old_commits
                    .into_iter()
                    .enumerate()
                    .map(|(idx, entry)| RangeDiffMatch {
                        changed: true,
                        old: Some((idx as u32 + 1, entry.commit)),
                        new: None,
                        title: entry.title,
                    })
                    .collect(),
            });
        }

        try_forward(
            || -> Result<RangeDiff> {
                let result = self.exec(
                    "range-diff",
                    [
                        "-s".into(),
                        format!("{}..{}", old.start.borrow(), old.end.borrow()),
                        format!("{}..{}", new.start.borrow(), new.end.borrow()),
                    ]
                    .iter(),
                )?;

                RangeDiff::parse(&result)
            },
            || "failed to obtain range-diff",
        )
    }
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub commit: Ref,
    pub title: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub struct RangeDiffMatchColumnWidths(usize, usize, usize, usize);
impl RangeDiffMatchColumnWidths {
    pub fn max(self, rhs: RangeDiffMatchColumnWidths) -> Self {
        Self(
            self.0.max(rhs.0),
            self.1.max(rhs.1),
            self.2.max(rhs.2),
            self.3.max(rhs.3),
        )
    }
}
impl Default for RangeDiffMatchColumnWidths {
    fn default() -> Self {
        Self(1, 1, 1, 1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RangeDiffMatch {
    pub changed: bool,
    pub old: Option<(u32, Ref)>,
    pub new: Option<(u32, Ref)>,
    pub title: Vec<u8>,
}
impl RangeDiffMatch {
    pub fn column_widths(&self) -> RangeDiffMatchColumnWidths {
        let old_idx = self.old.as_ref().map_or((1, 1), |(idx, hash)| {
            (format!("{idx}").len(), format!("{hash}").len())
        });
        let new_idx = self.new.as_ref().map_or((1, 1), |(idx, hash)| {
            (format!("{idx}").len(), format!("{hash}").len())
        });

        RangeDiffMatchColumnWidths(
            old_idx.0,
            old_idx.1,
            new_idx.0,
            new_idx.1,
        )
    }

    pub fn format(&self, widths: RangeDiffMatchColumnWidths) -> String {
        struct Column(usize, Option<String>);
        impl std::fmt::Display for Column {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match &self.1 {
                    Some(string) => write!(f, "{string:0$}", self.0),
                    None => write!(f, "{0:-<1$}", '-', self.0),
                }
            }
        }

        let change = match (self.changed, &self.old, &self.new) {
            (false, _, _) => "=",
            (true, Some(_), None) => "<",
            (true, None, Some(_)) => ">",
            _ => "!",
        };

        let (old_idx, old_hash) = self.old.as_ref().map_or((None, None), |(idx, hash)| {
            (Some(format!("{idx}")), Some(format!("{hash}")))
        });
        let (new_idx, new_hash) = self.new.as_ref().map_or((None, None), |(idx, hash)| {
            (Some(format!("{idx}")), Some(format!("{hash}")))
        });

        format!(
            "{}: {} {} {}: {} {}",
            Column(widths.0, old_idx),
            Column(widths.1, old_hash),
            change,
            Column(widths.2, new_idx),
            Column(widths.3, new_hash),
            String::from_utf8_lossy(&self.title)
        ).to_string()
    }
}

pub trait RangeDiffWriter {
    fn push_range_diff_match(&mut self, rdm: RangeDiffMatch);
}

#[derive(Debug)]
pub struct RangeDiff {
    pub matches: Vec<RangeDiffMatch>,
}
impl RangeDiff {
    fn parse_impl(buffer: &[u8]) -> Result<Self> {
        lazy_static! {
            static ref RE: Regex = Regex::new(
                r"(?-u)(?:(-+)|([0-9]+)): +(?:(-+)|([0-9a-f]+)) ([!<>=]) +(?:(-+)|([0-9]+)): +(?:(-+)|([0-9a-f]+)) +(.*)"
            ).unwrap();
        }

        let mut matches = Vec::new();
        for line in buffer.split(|&ch| ch == b'\n') {
            let line = trim_ascii(line);
            if line.is_empty() {
                continue;
            }

            let captures = RE
                .captures(line)
                .ok_or_else(|| format!("bad diff-range line\n{}", String::from_utf8_lossy(line)))?;

            fn get_side(
                captures: &regex::bytes::Captures,
                idx: usize,
            ) -> Result<Option<(u32, Ref)>> {
                let index_missing = captures.get(idx);
                let index_number = captures.get(idx + 1);
                let commit_missing = captures.get(idx + 2);
                let commit_hash = captures.get(idx + 3);

                if index_missing.is_some() != commit_missing.is_some() {
                    return Err("one of index and commit missing, but not both".into());
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

            let (changed, old_expected, new_expected) = match captures.get(5).unwrap().as_bytes() {
                b"=" => (false, true, true),
                b"!" => (true, true, true),
                b">" => (true, false, true),
                b"<" => (true, true, false),
                other => {
                    return Err(format!(
                        "bad change indicator '{}'",
                        String::from_utf8_lossy(other)
                    )
                    .into())
                }
            };

            if old.is_some() != old_expected || new.is_some() != new_expected {
                return Err("change indicator doesn't match the shown commits".into());
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
        assert_eq!(
            rd.matches[0],
            RangeDiffMatch {
                changed: true,
                old: Some((1, Ref::new("31b5c003"))),
                new: Some((1, Ref::new("d73727e2"))),
                title: (*b"title foo").into(),
            }
        );
        assert_eq!(
            rd.matches[1],
            RangeDiffMatch {
                changed: true,
                old: None,
                new: Some((2, Ref::new("98ad5553"))),
                title: (*b"title blah").into(),
            }
        );
        assert_eq!(
            rd.matches[2],
            RangeDiffMatch {
                changed: true,
                old: Some((3, Ref::new("01234567"))),
                new: None,
                title: (*b"blub").into(),
            }
        );
        assert_eq!(
            rd.matches[3],
            RangeDiffMatch {
                changed: false,
                old: Some((2, Ref::new("89abcdef"))),
                new: Some((3, Ref::new("fedc3210"))),
                title: (*b"another").into(),
            }
        );

        Ok(())
    }

    #[test]
    fn range_diff_long() -> Result<()> {
        // With 10 or more commits, the number of spaces changes due to the
        // column alignment. This test simply checks that parsing succeeds.
        let range_diff_text = "\
            1:  ce2d771c8 =  1:  ce2d771c8 Some title\n\
            2:  3048b9cd5 =  2:  3048b9cd5 Some title\n\
            3:  46c6da7f7 =  3:  46c6da7f7 Some title\n\
            4:  ef3268f45 =  4:  ef3268f45 Some title\n\
            5:  0dd787c71 =  5:  0dd787c71 Some title\n\
            6:  b3c0f3c0b =  6:  b3c0f3c0b Some title\n\
            7:  fcd2a46ed =  7:  fcd2a46ed Some title\n\
            8:  87217884d =  8:  87217884d Some title\n\
            9:  c06759892 =  9:  c06759892 Some title\n\
            -:  --------- > 10:  22d6987c2 Some title\n\
            -:  --------- > 11:  5595185f8 Some title\n\
        ";

        let _ = RangeDiff::parse(range_diff_text.as_bytes())?;

        Ok(())
    }
}
