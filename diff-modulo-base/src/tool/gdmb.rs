// SPDX-License-Identifier: MIT

use std::ops::Range;

use clap::Parser;

use crate::*;
use git_core::{RangeDiffWriter, Ref};
use utils::Result;
use diff::ChunkWriter;

#[derive(Parser, Debug)]
pub struct GitDiffModuloBaseOptions {
    /// Combine the diff of all commits in a range, instead of showing per-commit diffs.
    #[clap(long)]
    pub combined: bool,
}

#[derive(Parser, Debug)]
pub struct GitDiffModuloBaseArgs {
    pub base: Option<String>,
    pub old: Option<String>,
    pub new: Option<String>,

    #[clap(flatten)]
    pub options: GitDiffModuloBaseOptions,
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

pub trait DiffModuloBaseWriter : ChunkWriter + RangeDiffWriter {}
impl<T: ChunkWriter + RangeDiffWriter> DiffModuloBaseWriter for T {}

pub fn git_diff_modulo_base(
    mut args: GitDiffModuloBaseArgs,
    repo: git_core::Repository,
    writer: &mut dyn DiffModuloBaseWriter,
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

    match (old, new) {
        (old @ RevSpec::Range(_, _), new @ RevSpec::Range(_, _)) => {
            if args.options.combined {
                git::diff_ranges_full(&repo, old.to_range(), new.to_range(), writer)?;
            } else {
                let range_diff = repo.range_diff(old.to_range(), new.to_range())?;

                for rd_match in range_diff.matches {
                    let changed = rd_match.changed;
                    let old = rd_match.old.as_ref().map(|(_, commit)| commit.clone());
                    let new = rd_match.new.as_ref().map(|(_, commit)| commit.clone());

                    writer.push_range_diff_match(rd_match);

                    if changed {
                        git::diff_optional_commits(&repo, old, new, writer)?;
                    }
                }
            }
        }
        (RevSpec::Commit(old), RevSpec::Commit(new)) => {
            git::diff_commits(&repo, &old, &new, writer)?;
        }
        _ => return Err("old and new must either both refer to commits or both to ranges".into()),
    };

    Ok(())
}
