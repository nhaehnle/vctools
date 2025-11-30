// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;
use std::fmt::Write;

use diff_modulo_base::git_core::{self, Ref};
use diff_modulo_base::tool::{self, GitDiffModuloBaseArgs, GitDiffModuloBaseOptions};
use regex::Regex;
use vctuik::{
    event::KeyCode,
    pager::{Pager, PagerState, RichPagerSource, StringPagerSource},
    prelude::*,
    state::Builder,
};

use crate::github::api;
use crate::{github::connections::Connections, CompletePullRequest};

use super::{actions, diff_pager::DiffPagerSource};

#[derive(Debug)]
struct CommentOrReview {
    user: String,
    submitted_at: chrono::DateTime<chrono::Utc>,
    body: String,
    commit_id: Option<String>,
    review_state: Option<api::ReviewState>,
}
impl CommentOrReview {
    fn is_significant_review(&self) -> bool {
        if let Some(state) = &self.review_state {
            state.is_significant()
        } else {
            false
        }
    }
}

fn normalize_comments_and_reviews(reviews: Vec<api::Review>, comments: Vec<api::Comment>) -> Vec<CommentOrReview> {
    let mut items: Vec<CommentOrReview> = Vec::new();

    for review in reviews {
        let submitted_at = review.submitted_at().unwrap();
        items.push(CommentOrReview {
            user: review.user.login,
            submitted_at: submitted_at,
            body: review.body,
            commit_id: review.commit_id,
            review_state: Some(review.state),
        });
    }

    for comment in comments {
        let created_at = comment.created_at().unwrap();
        items.push(CommentOrReview {
            user: comment.user.login,
            submitted_at: created_at,
            body: comment.body,
            commit_id: None,
            review_state: None,
        });
    }

    items.sort_by(|a, b| a.submitted_at.cmp(&b.submitted_at));
    items
}

#[derive(Debug, Default)]
struct ReviewState {
    options: GitDiffModuloBaseOptions,
    head_pager: RichPagerSource<'static>,
    diff_pager: DiffPagerSource,
    pager_state: PagerState,
    pr: Option<CompletePullRequest>,
    dmb_args: Option<GitDiffModuloBaseArgs>,

    /// Whether to rebuild on the next frame (but let the next frame be triggered
    /// by asynchronous API / Git fetch completion).
    need_rebuild: bool,
}
impl ReviewState {
    fn update(
        &mut self,
        connections: &mut Connections,
        ep: &dyn git_core::ExecutionProvider,
        pr: GCow<'_, CompletePullRequest>,
    ) {
        let mut keep_pager_state = false;

        if let Some(old_pr) = &mut self.pr {
            if *old_pr != *pr {
                *old_pr = pr.into_owned();
            } else {
                let options_changed = self.dmb_args.as_ref().is_some_and(|args| args.options != self.options);
                if !self.need_rebuild && !options_changed {
                    // Just re-use the cached pager data.
                    return;
                }

                // If the PR and options stays the same and we're just retrying
                // because an API or Git fetch timed out last time around, then
                // we can keep the same pager state.
                if !options_changed {
                    keep_pager_state = true;
                }
            }
        } else {
            self.pr = Some(pr.into_owned());
        }

        self.head_pager = RichPagerSource::new();
        self.diff_pager = DiffPagerSource::new();
        if !keep_pager_state {
            self.pager_state = PagerState::default();
        }
        self.need_rebuild = false;

        if let Err(err) = self.build(connections, ep) {
            let mut text = String::new();
            if ep.timed_out() {
                writeln!(&mut text, "Generating diff... {err}").unwrap();
                self.need_rebuild = true;
            } else {
                writeln!(&mut text, "Error loading review: {err}").unwrap();
                self.need_rebuild = false;
            }
            self.head_pager.add_child(StringPagerSource::new(text));
        }
    }

    fn build(
        &mut self,
        connections: &mut Connections,
        ep: &dyn git_core::ExecutionProvider,
    ) -> Result<()> {
        // Fire off all requests.
        let pr = self.pr.as_ref().unwrap();
        let mut client = connections.client(&pr.api.host)?.borrow_mut();
        let client_ref = client.access();
        let pull = client_ref.pull(&pr.api.owner, &pr.api.name, pr.id);
        let reviews = client_ref.reviews(&pr.api.owner, &pr.api.name, pr.id);
        let comments = client_ref.issue_comments(&pr.api.owner, &pr.api.name, pr.id);

        let Some(pull) = pull.ok_or_pending()? else {
            self.head_pager.add_child(StringPagerSource::new(
                "Loading pull request...",
            ));
            self.need_rebuild = true;
            return Ok(());
        };

        {
            let mut header = String::new();
            let state = match pull.state {
                api::PullState::Open => {
                    if pull.draft {
                        "⚪ Draft"
                    } else {
                        "🟢 Open"
                    }
                }
                api::PullState::Closed => {
                    if pull.merged {
                        "🟣 Merged"
                    } else {
                        "🔴 Closed"
                    }
                }
                api::PullState::Other => "❓ Unknown",
            };
            writeln!(
                &mut header,
                "Pull Request {}/{}#{} ({})",
                pr.api.owner, pr.api.name, pr.id, pull.html_url,
            )?;
            writeln!(&mut header, "Title:   {}", pull.title)?;
            writeln!(&mut header, "Author:  @{}", pull.user.login)?;
            writeln!(&mut header, "State:   {}", state)?;
            self.head_pager.add_child(StringPagerSource::new(header));
        }

        let reviews = reviews.ok_or_pending()?;
        let comments = comments.ok_or_pending()?;

        let main_comments = reviews.zip(comments).map(|(r, c)| normalize_comments_and_reviews(r, c));

        if let Some(main_comments) = &main_comments {
            let mut header = String::new();

            if main_comments.is_empty() {
                writeln!(&mut header, "No reviews or comments yet")?;
            } else {
                // Keep only the most recent review or comment by each user,
                // except we also keep the most significant reviews
                // (approved / changes requested).
                let mut filtered: Vec<&CommentOrReview> = Vec::new();
                let mut max_user_len = 0;
                for c in main_comments.iter().rev() {
                    if !filtered.iter().any(|f| {
                        f.user == c.user &&
                        (f.is_significant_review() || !c.is_significant_review())
                    }) {
                        filtered.push(c);
                        max_user_len = max_user_len.max(c.user.len());
                    }
                }

                writeln!(&mut header, "Most recent reviews and comments by user:")?;
                for c in filtered.into_iter().rev() {
                    let state = match c.review_state {
                        Some(api::ReviewState::Approved) => "✅",
                        Some(api::ReviewState::ChangesRequested) => "❌",
                        Some(api::ReviewState::Commented) | None => "💬",
                        Some(api::ReviewState::Other) => "❓",
                    };
                    writeln!(
                        &mut header,
                        "  @{:<max_user_len$} {} {}{}",
                        c.user,
                        state,
                        c.submitted_at,
                        if let Some(commit_id) = c.commit_id.as_ref() {
                            format!(" (at {})", commit_id)
                        } else {
                            String::new()
                        }
                    )?;
                }
            }

            self.head_pager.add_child(StringPagerSource::new(header));
        } else {
            self.head_pager.add_child(StringPagerSource::new("Loading reviews and comments..."));
            self.need_rebuild = true;
        }

        {
            let mut header = String::new();
            writeln!(&mut header, "Current head:       {} ({})", pull.head.ref_, pull.head.sha)?;
            writeln!(&mut header, "Target branch:      {} ({})", pull.base.ref_, pull.base.sha)?;
            self.head_pager.add_child(StringPagerSource::new(header));
        }

        let most_recent_review = main_comments
            .iter()
            .flatten()
            .rev()
            .filter(|review| review.commit_id.is_some())
            .find(|review| review.user == client.host().user);

        let refs: Vec<_> = [&pull.head.sha, &pull.base.sha]
            .into_iter()
            .chain(most_recent_review.iter().map(|review| review.commit_id.as_ref().unwrap()))
            .map(|sha| Ref::new(sha))
            .collect();
        pr.git.repository.fetch_missing(ep, &pr.git.remote, &refs)?;

        let old = if let Some(review) = most_recent_review {
            review.commit_id.clone().unwrap()
        } else {
            pr.git
                .repository
                .merge_base(ep, &Ref::new(&pull.base.sha), &Ref::new(&pull.head.sha))?
                .name
        };

        let dmb_args = tool::GitDiffModuloBaseArgs {
            base: Some(pull.base.sha),
            old: Some(old),
            new: Some(pull.head.sha),
            options: self.options,
        };

        tool::git_diff_modulo_base(&dmb_args, &pr.git.repository, ep, &mut self.diff_pager)?;

        self.dmb_args = Some(dmb_args);
        Ok(())
    }
}

pub struct Review<'build> {
    pr: GCow<'build, CompletePullRequest>,
    ep: &'build dyn git_core::ExecutionProvider,
    options: Option<&'build mut GitDiffModuloBaseOptions>,
    search: Option<&'build Regex>,
}
impl<'build> Review<'build> {
    pub fn new(
        ep: &'build dyn git_core::ExecutionProvider,
        pr: impl Into<GCow<'build, CompletePullRequest>>,
    ) -> Self {
        Self {
            pr: pr.into(),
            ep,
            options: Default::default(),
            search: None,
        }
    }

    pub fn search(self, search: &'build Regex) -> Self {
        Self {
            search: Some(search),
            ..self
        }
    }

    pub fn maybe_search(self, search: Option<&'build Regex>) -> Self {
        Self { search, ..self }
    }

    pub fn options(self, options: &'build mut GitDiffModuloBaseOptions) -> Self {
        Self {
            options: Some(options),
            ..self
        }
    }

    pub fn build(self, builder: &mut Builder, connections: &mut Connections) {
        let state_id = builder.add_state_id("review");
        let state: &mut ReviewState = builder.get_state(state_id);

        if let Some(options) = &self.options {
            if **options != state.options {
                state.options = **options;
                state.need_rebuild = true;
            }
        }

        builder
            .nest()
            .id(state_id)
            .build(|builder| {
                let has_focus = builder.check_group_focus(state_id);
                if has_focus {
                    if builder.on_key_press(KeyCode::Char('C')) {
                        state.options.combined = !state.options.combined;
                        if let Some(options) = self.options {
                            options.combined = state.options.combined;
                        }
                        state.need_rebuild = true;
                    } else if builder.on_key_press(KeyCode::Char('d')) {
                        state.diff_pager.toggle_mode();
                    }
                }

                state.update(connections, self.ep, self.pr);

                let mut pager = RichPagerSource::new();
                pager.add_child_ref(&state.head_pager);
                pager.add_child_ref(&state.diff_pager);

                let mut pager = Pager::new(&pager);
                if let Some(regex) = self.search {
                    pager = pager.search(Cow::Borrowed(regex));
                }
                let mut pager_result = pager.build_with_state(builder, "pager", &mut state.pager_state);

                if has_focus {
                    if let Some(search) = builder.on_custom::<actions::Search>() {
                        pager_result.search(&search.0, true);
                        builder.need_refresh();
                    }
                }
            });
    }
}
