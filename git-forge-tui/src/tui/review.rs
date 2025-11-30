// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;
use std::fmt::Write;
use std::result::Result as StdResult;

use diff_modulo_base::git_core::{self, Ref};
use diff_modulo_base::tool::{self, GitDiffModuloBaseArgs, GitDiffModuloBaseOptions};
use ratatui::text::Text;
use regex::Regex;
use vctuik::label::{add_multiline_label, add_text_label};
use vctuik::{
    event::KeyCode,
    pager::{Pager, PagerState},
    prelude::*,
    state::Builder,
};

use crate::github::api;
use crate::{github::connections::Connections, CompletePullRequest};

use super::{actions, diff_pager::DiffPagerSource};

#[derive(Debug)]
struct Inner {
    pr: CompletePullRequest,
    header: String,
    dmb_args: Option<GitDiffModuloBaseArgs>,
    pager: StdResult<(DiffPagerSource, PagerState), String>,
    timed_out: bool,
}

#[derive(Debug, Default)]
struct ReviewState {
    options: GitDiffModuloBaseOptions,
    inner: Option<Inner>,
}
impl ReviewState {
    fn update(
        &mut self,
        connections: &mut Connections,
        ep: &dyn git_core::ExecutionProvider,
        pr: GCow<'_, CompletePullRequest>,
    ) {
        let result = || -> Result<_> {
            let mut client = connections.client(&pr.api.host)?.borrow_mut();
            let client_ref = client.access();
            let pull = client_ref.pull(&pr.api.owner, &pr.api.name, pr.id).ok()?;
            let reviews = client_ref
                .reviews(&pr.api.owner, &pr.api.name, pr.id)
                .ok()?;

            Ok((client.host().user.to_string(), pull, reviews))
        }();
        let Ok((user, pull, reviews)) = result else {
            let err = result.err().unwrap();
            self.inner = Some(Inner {
                pr: pr.into_owned(),
                header: String::new(),
                dmb_args: None,
                pager: Err(err.to_string()),
                timed_out: ep.timed_out(),
            });
            return;
        };

        let most_recent_review = reviews
            .iter()
            .rev()
            .filter(|review| review.commit_id.is_some())
            .find(|review| review.user.login == user);

        let header = || -> Result<_> {
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

            if reviews.is_empty() {
                writeln!(&mut header, "No reviews yet")?;
            } else {
                let mut reviews_by_user: Vec<&api::Review> = Vec::new();
                let mut max_user_len = 0;
                for review in &reviews {
                    let user = &review.user.login;
                    if let Some(r) = reviews_by_user.iter_mut().find(|r| r.user.login == *user) {
                        *r = review;
                    } else {
                        reviews_by_user.push(review);
                        max_user_len = max_user_len.max(user.len());
                    }
                }

                writeln!(&mut header, "Most recent reviews:")?;
                for review in reviews_by_user {
                    let state = match review.state {
                        api::ReviewState::Approved => "✅",
                        api::ReviewState::ChangesRequested => "❌",
                        api::ReviewState::Commented => "💬",
                        api::ReviewState::Other => "❓",
                    };
                    writeln!(
                        &mut header,
                        "  @{:<max_user_len$} {} {}{}",
                        review.user.login,
                        state,
                        review.submitted_at,
                        if let Some(commit_id) = review.commit_id.as_ref() {
                            format!(" (at {})", commit_id)
                        } else {
                            String::new()
                        }
                    )?;
                }
            }

            if let Some(review) = &most_recent_review {
                writeln!(&mut header, "  Most recent review: {}", review.commit_id.as_ref().unwrap())?;
            }
            writeln!(&mut header, "Current head:       {} ({})", pull.head.ref_, pull.head.sha)?;
            writeln!(&mut header, "Target branch:      {} ({})", pull.base.ref_, pull.base.sha)?;
            Ok(header)
        }()
        .unwrap();

        let result = || -> Result<_> {
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

            Ok(tool::GitDiffModuloBaseArgs {
                base: Some(pull.base.sha),
                old: Some(old),
                new: Some(pull.head.sha),
                options: self.options,
            })
        }();
        let Ok(dmb_args) = result else {
            let err = result.err().unwrap();
            self.inner = Some(Inner {
                pr: pr.into_owned(),
                header,
                dmb_args: None,
                pager: Err(err.to_string()),
                timed_out: ep.timed_out(),
            });
            return;
        };

        if self.inner.as_ref().is_some_and(|inner| {
            !inner.timed_out
                && inner.pr == *pr
                && inner
                    .dmb_args
                    .as_ref()
                    .is_some_and(|args| dmb_args == *args)
        }) {
            return;
        }

        let header_copy = header.clone();
        let pager = || -> Result<_> {
            let mut pager_source = DiffPagerSource::new();
            pager_source.push_header(header_copy);
            tool::git_diff_modulo_base(&dmb_args, &pr.git.repository, ep, &mut pager_source)?;
            Ok((pager_source, PagerState::default()))
        }();
        let pager = match pager {
            Ok(pager) => Ok(pager),
            Err(err) => Err(err.to_string()),
        };

        self.inner = Some(Inner {
            pr: pr.into_owned(),
            header,
            dmb_args: Some(dmb_args),
            pager,
            timed_out: ep.timed_out(),
        });
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
        let state_outer: &mut ReviewState = builder.get_state(state_id);

        if let Some(options) = &self.options {
            state_outer.options = **options;
        }

        state_outer.update(connections, self.ep, self.pr);

        let state = state_outer.inner.as_mut().unwrap();

        builder
            .nest()
            .id(state_id)
            .build(|builder| match &mut state.pager {
                Ok((pager_source, pager_state)) => {
                    let has_focus = builder.check_group_focus(state_id);

                    let mut pager = Pager::new(pager_source);
                    if let Some(regex) = self.search {
                        pager = pager.search(Cow::Borrowed(regex));
                    }
                    let mut pager_result = pager.build_with_state(builder, "pager", pager_state);

                    if has_focus {
                        if builder.on_key_press(KeyCode::Char('C')) {
                            state_outer.options.combined = !state_outer.options.combined;
                            if let Some(options) = self.options {
                                options.combined = state_outer.options.combined;
                            }
                            builder.need_refresh();
                        } else if builder.on_key_press(KeyCode::Char('d')) {
                            std::mem::drop(pager_result);
                            pager_source.toggle_mode();
                            builder.need_refresh();
                        } else if let Some(search) = builder.on_custom::<actions::Search>() {
                            pager_result.search(&search.0, true);
                            builder.need_refresh();
                        }
                    }
                }
                Err(err) => {
                    if !state.header.is_empty() {
                        add_multiline_label(builder, &state.header);
                    }
                    add_text_label(
                        builder,
                        Text::raw(format!("Error: {err}")).style(builder.theme().pane_text.error),
                    );
                    builder.add_slack();
                }
            });
    }
}
