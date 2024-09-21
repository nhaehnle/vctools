// SPDX-License-Identifier: MIT

use std::{fmt::Display, ops::Range};

use clap::Parser;
use termcolor::{Color, ColorSpec};

use crate::*;
use git_core::Ref;
use utils::Result;

#[derive(Parser, Debug)]
pub struct GitDiffModuloBaseOptions {
    pub base: Option<String>,
    pub old: Option<String>,
    pub new: Option<String>,

    /// Combine the diff of all commits in a range, instead of showing per-commit diffs.
    #[clap(long)]
    pub combined: bool,
}

#[derive(Debug, Clone)]
enum RevSpec {
    Commit(Ref),
    Range(Ref, Ref),
}
impl RevSpec {
    fn to_range(self) -> Range<Ref> {
        match self {
            Self::Commit(commit) => commit.first_parent()..commit,
            Self::Range(start, end) => start..end,
        }
    }
}

fn parse_rev_or_range(name: &str) -> Result<RevSpec> {
    if let Some((start, end)) = name.split_once("..") {
        if end.find("..").is_some() {
            return Err("rev or range with multiple ..".into());
        }
        Ok(RevSpec::Range(Ref::new(start), Ref::new(end)))
    } else {
        let commit = Ref::new(name);
        Ok(RevSpec::Commit(commit))
    }
}

pub fn git_diff_modulo_base(
    mut args: GitDiffModuloBaseOptions,
    repo: git_core::Repository,
    out: &mut dyn termcolor::WriteColor,
) -> Result<()> {
    if args.old.is_none() {
        return Err("need both an old and a new revision".into());
    }

    if args.new.is_none() {
        (args.base, args.old, args.new) = (None, args.base, args.old)
    }

    let base = match args.base {
        Some(s) => Some(parse_rev_or_range(&s)?),
        None => None,
    };
    let mut old = parse_rev_or_range(&args.old.unwrap())?;
    let mut new = parse_rev_or_range(&args.new.unwrap())?;

    if let Some(base) = base {
        let RevSpec::Commit(base) = base else {
            return Err("BASE must refer to a single commit".into());
        };
        let (RevSpec::Commit(old_ref), RevSpec::Commit(new_ref)) = (old, new) else {
            return Err("when BASE is used, both OLD and NEW must refer to a single commit".into());
        };

        let old_base = repo.merge_base(&base, &old_ref)?;
        let new_base = repo.merge_base(&base, &new_ref)?;

        old = RevSpec::Range(old_base, repo.rev_parse(&old_ref)?);
        new = RevSpec::Range(new_base, repo.rev_parse(&new_ref)?);
    }

    let mut writer = diff_color::Writer::new(out);

    match (old, new) {
        (old @ RevSpec::Range(_, _), new @ RevSpec::Range(_, _)) => {
            if args.combined {
                git::diff_ranges_full(&repo, old.to_range(), new.to_range(), &mut writer)?;
            } else {
                let range_diff = repo.range_diff(old.to_range(), new.to_range())?;

                let match_lines: Vec<_> = range_diff
                    .matches
                    .iter()
                    .map(|rd_match| {
                        let (old_idx, old_hash) = rd_match
                            .old
                            .as_ref()
                            .map(|(idx, hash)| (Some(format!("{idx}")), Some(format!("{hash}"))))
                            .unwrap_or((None, None));
                        let (new_idx, new_hash) = rd_match
                            .new
                            .as_ref()
                            .map(|(idx, hash)| (Some(format!("{idx}")), Some(format!("{hash}"))))
                            .unwrap_or((None, None));
                        let change = match (rd_match.changed, &rd_match.old, &rd_match.new) {
                            (false, _, _) => "=",
                            (true, Some(_), None) => "<",
                            (true, None, Some(_)) => ">",
                            _ => "!",
                        };
                        (old_idx, old_hash, change, new_idx, new_hash)
                    })
                    .collect();

                fn max_len<'a, I: Iterator<Item = &'a Option<String>>>(iter: I) -> usize {
                    iter.filter_map(|it| it.as_ref().map(|s| s.len()))
                        .max()
                        .unwrap_or(1)
                }

                let len = (
                    max_len(match_lines.iter().map(|row| &row.0)),
                    max_len(match_lines.iter().map(|row| &row.1)),
                    max_len(match_lines.iter().map(|row| &row.3)),
                    max_len(match_lines.iter().map(|row| &row.4)),
                );

                struct Column(usize, Option<String>);
                impl Display for Column {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        match &self.1 {
                            Some(string) => write!(f, "{string:0$}", self.0),
                            None => write!(f, "{0:-<1$}", '-', self.0),
                        }
                    }
                }

                for (rd_match, (old_idx, old_hash, change, new_idx, new_hash)) in
                    range_diff.matches.iter().zip(match_lines.into_iter())
                {
                    writer.out.set_color(
                        ColorSpec::new()
                            .set_bg(Some(Color::Cyan))
                            .set_fg(Some(Color::Black)),
                    )?;
                    writeln!(
                        writer.out,
                        "{}: {} {} {}: {} {}",
                        Column(len.0, old_idx),
                        Column(len.1, old_hash),
                        change,
                        Column(len.2, new_idx),
                        Column(len.3, new_hash),
                        String::from_utf8_lossy(&rd_match.title)
                    )?;

                    if rd_match.changed {
                        let old = rd_match.old.as_ref().map(|(_, commit)| commit);
                        let new = rd_match.new.as_ref().map(|(_, commit)| commit);
                        git::diff_optional_commits(&repo, old, new, &mut writer)?;
                    }
                }
            }
        }
        (RevSpec::Commit(old), RevSpec::Commit(new)) => {
            git::diff_commits(&repo, &old, &new, &mut writer)?;
        }
        _ => return Err("old and new must either both refer to commits or both to ranges".into()),
    };

    Ok(())
}
