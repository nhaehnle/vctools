// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use crate::*;
use diff::{ChunkWriter, ChunkWriterExt};
use git_core::{ExecutionProvider, Range, Ref};
use utils::Result;

fn diff_ranges_full_impl(
    repo: &git_core::Repository,
    ep: &dyn ExecutionProvider,
    old: Option<Range<&Ref>>,
    new: Option<Range<&Ref>>,
    writer: &mut dyn ChunkWriter,
) -> Result<()> {
    let mut buffer = diff::Buffer::new();

    fn get_diff(
        ep: &dyn ExecutionProvider,
        buffer: &mut diff::Buffer,
        repo: &git_core::Repository,
        range: &Option<Range<&Ref>>,
    ) -> Result<diff::Diff> {
        if let Some(range) = range {
            let diff_text = buffer.insert(&repo.diff(ep, range.clone(), None)?)?;
            Ok(diff::Diff::parse(&buffer, diff_text)?)
        } else {
            Ok(diff::Diff::new(diff::DiffOptions::default()))
        }
    }

    let base_old_diff = get_diff(ep, &mut buffer, repo, &old)?;
    let base_new_diff = get_diff(ep, &mut buffer, repo, &new)?;

    let target_diff = match (old, new) {
        (Some(old), Some(new)) => {
            let mut paths: HashSet<&[u8]> = HashSet::new();
            for file in base_old_diff.iter_files().chain(base_new_diff.iter_files()) {
                if let diff::FileName::Name(name) = &file.new_name {
                    paths.insert(&name);
                }
            }

            // Sort paths to ensure cacheability and a deterministic diff result.
            let mut paths: Vec<&[u8]> = paths.into_iter().collect();
            paths.sort();
            let target = buffer.insert(&repo.diff(ep, old.end..new.end, Some(&paths))?)?;
            diff::Diff::parse(&buffer, target)?
        }
        (Some(_), _) => diff::reverse(&base_old_diff),
        (_, Some(_)) => base_new_diff.clone(),
        _ => panic!("at least one range needs to be provided"),
    };

    diff::diff_modulo_base(&buffer, target_diff, &base_old_diff, &base_new_diff, writer)?;

    Ok(())
}

/// Produce a base-reduced diff between the two given ranges, one of which may
/// be empty (i.e., no change).
pub fn diff_optional_ranges_full<R>(
    repo: &git_core::Repository,
    ep: &dyn ExecutionProvider,
    old: Option<Range<R>>,
    new: Option<Range<R>>,
    writer: &mut dyn ChunkWriter,
) -> Result<()>
where
    R: std::borrow::Borrow<Ref>,
{
    diff_ranges_full_impl(
        repo,
        ep,
        old.as_ref()
            .map(|range| range.start.borrow()..range.end.borrow()),
        new.as_ref()
            .map(|range| range.start.borrow()..range.end.borrow()),
        writer,
    )
}

/// Produce a base-reduced diff between the two given ranges.
pub fn diff_ranges_full<R>(
    repo: &git_core::Repository,
    ep: &dyn ExecutionProvider,
    old: Range<R>,
    new: Range<R>,
    writer: &mut dyn ChunkWriter,
) -> Result<()>
where
    R: std::borrow::Borrow<Ref>,
{
    diff_ranges_full_impl(
        repo,
        ep,
        Some(old.start.borrow()..old.end.borrow()),
        Some(new.start.borrow()..new.end.borrow()),
        writer,
    )
}

fn diff_optional_commits_impl(
    repo: &git_core::Repository,
    ep: &dyn ExecutionProvider,
    old: Option<&Ref>,
    new: Option<&Ref>,
    writer: &mut dyn ChunkWriter,
) -> Result<()> {
    fn get_meta(
        buffer: &mut diff::Buffer,
        repo: &git_core::Repository,
        ep: &dyn ExecutionProvider,
        commit: Option<&Ref>,
        name: &[u8],
    ) -> Result<(diff::DiffRef, diff::DiffRef)> {
        if let Some(commit) = commit {
            let show_options = git_core::ShowOptions {
                show_patch: false,
                skip_commit_id: true,
                ..Default::default()
            };

            Ok((
                buffer.insert(&repo.show_commit(ep, commit, &show_options)?)?,
                buffer.insert(name)?,
            ))
        } else {
            Ok((diff::DiffRef::default(), buffer.insert(b"/dev/null")?))
        }
    }

    let mut buffer = diff::Buffer::new();
    let (old_meta, old_meta_name) = get_meta(&mut buffer, repo, ep, old, b"a/commit-meta")?;
    let (new_meta, new_meta_name) = get_meta(&mut buffer, repo, ep, new, b"a/commit-meta")?;

    let meta_diff = diff::diff_file(
        &buffer,
        old_meta_name,
        new_meta_name,
        old_meta,
        new_meta,
        &diff::DiffOptions {
            strip_path_components: 1,
            ..Default::default()
        },
        diff::DiffAlgorithm::default(),
    )?;

    struct DelayedMetaWriter<'a> {
        writer: &'a mut dyn ChunkWriter,
        meta_diff_buffer: &'a diff::Buffer,
        meta_diff: Option<diff::DiffFile>,
    }
    impl<'a> ChunkWriter for DelayedMetaWriter<'a> {
        fn push_chunk(&mut self, chunk: diff::Chunk) {
            if let Some(meta_diff) = self.meta_diff.take() {
                meta_diff.render_full_body(
                    &self.meta_diff_buffer,
                    &mut self.writer.with_context(diff::Context::CommitMessage),
                );
            }
            self.writer.push_chunk(chunk);
        }
    }

    let mut delayed_meta_writer = DelayedMetaWriter {
        writer,
        meta_diff_buffer: &buffer,
        meta_diff: Some(meta_diff),
    };

    diff_optional_ranges_full(
        repo,
        ep,
        old.map(|commit| commit.first_parent()..commit.clone()),
        new.map(|commit| commit.first_parent()..commit.clone()),
        &mut delayed_meta_writer,
    )?;

    // Handle the case where only the commit meta (e.g. message) has changed.
    if let Some(meta_diff) = delayed_meta_writer.meta_diff.take() {
        if !meta_diff.is_unchanged() {
            meta_diff.render_full_body(
                &buffer,
                &mut delayed_meta_writer
                    .writer
                    .with_context(diff::Context::CommitMessage),
            );
        }
    }

    Ok(())
}

/// Produce a base-reduced diff between the two given commits; this includes
/// diffs between the commit messages. Either side can be None, which will
/// produce a diff as if that side had an empty commit without metadata.
pub fn diff_optional_commits<R>(
    repo: &git_core::Repository,
    ep: &dyn ExecutionProvider,
    old: Option<R>,
    new: Option<R>,
    writer: &mut dyn ChunkWriter,
) -> Result<()>
where
    R: std::borrow::Borrow<Ref>,
{
    diff_optional_commits_impl(
        repo,
        ep,
        old.as_ref().map(|old| old.borrow()),
        new.as_ref().map(|new| new.borrow()),
        writer,
    )
}

/// Produce a base-reduced diff between the two given commits; this includes
/// diffs between the commit messages.
pub fn diff_commits(
    repo: &git_core::Repository,
    ep: &dyn ExecutionProvider,
    old: &Ref,
    new: &Ref,
    writer: &mut dyn ChunkWriter,
) -> Result<()> {
    let show_options = git_core::ShowOptions {
        show_patch: false,
        skip_commit_id: true,
        ..Default::default()
    };

    let mut buffer = diff::Buffer::new();
    let old_meta = buffer.insert(&repo.show_commit(ep, old, &show_options)?)?;
    let new_meta = buffer.insert(&repo.show_commit(ep, new, &show_options)?)?;
    let old_meta_name = buffer.insert(b"a/commit-meta")?;
    let new_meta_name = buffer.insert(b"b/commit-meta")?;

    let meta_diff = diff::diff_file(
        &buffer,
        old_meta_name,
        new_meta_name,
        old_meta,
        new_meta,
        &diff::DiffOptions {
            strip_path_components: 1,
            ..Default::default()
        },
        diff::DiffAlgorithm::default(),
    )?;

    meta_diff.render_full_body(
        &buffer,
        &mut writer.with_context(diff::Context::CommitMessage),
    );
    diff_ranges_full(
        repo,
        ep,
        &old.first_parent()..old,
        &new.first_parent()..new,
        writer,
    )
}
