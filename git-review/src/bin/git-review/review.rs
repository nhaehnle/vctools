// SPDX-License-Identifier: GPL-3.0-or-later

use std::{borrow::Cow, ops::Range};
use std::fmt::Write;
use std::result::Result as StdResult;

use diff_modulo_base::git_core::Ref;
use diff_modulo_base::{diff, git_core::{self, Repository}, tool::{self, GitDiffModuloBaseArgs, GitDiffModuloBaseOptions}};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use regex::Regex;
use vctuik::label::{add_multiline_label, add_text_label};
use vctuik::{
    event::KeyCode, pager::{Pager, PagerState}, prelude::*, state::Builder
};

use git_review::connections::Connections;

use crate::{actions, diff_pager::DiffPagerSource};

#[derive(Debug)]
struct Inner {
    pr: PullRequest,
    header: String,
    dmb_args: Option<GitDiffModuloBaseArgs>,
    pager: StdResult<(DiffPagerSource, PagerState), String>,
}

#[derive(Debug, Default)]
struct ReviewState {
    options: GitDiffModuloBaseOptions,
    inner: Option<Inner>,
}
impl ReviewState {
    fn update(&mut self, connections: &mut Connections, pr: GCow<'_, PullRequest>, options: GitDiffModuloBaseOptions) {
        let result = || -> Result<_> {
            let url = pr.repository.get_url(&pr.remote)?;

            let Some(hostname) = url.hostname() else {
                Err(format!("cannot find hostname for {url}"))?
            };
            let Some((organization, gh_repo)) = url.github_path() else {
                Err(format!("cannot parse {url} as a GitHub repository"))?
            };

            let client = connections.client(hostname.to_owned())?;
            let client_ref = client.access();
            let pull = client_ref.pull(organization, gh_repo, pr.id).ok()?;
            let reviews = client_ref.reviews(organization, gh_repo, pr.id).ok()?;

            Ok((client.host().user.to_string(), organization.to_string(), gh_repo.to_string(), pull, reviews))
        }();
        let Ok((user, organization, gh_repo, pull, reviews)) = result else {
            let err = result.err().unwrap();
            self.inner = Some(Inner {
                pr: pr.into_owned(),
                header: String::new(),
                dmb_args: None,
                pager: Err(err.to_string()),
            });
            return;
        };

        let most_recent_review = reviews
            .into_iter()
            .rev()
            .find(|review| review.user.login == user);

        let header = || -> Result<_> {
            let mut header = String::new();
            writeln!(
                &mut header,
                "Review {}/{}#{}",
                organization, gh_repo, pr.id
            )?;
            if let Some(review) = &most_recent_review {
                writeln!(
                    &mut header,
                    "  Most recent review: {}",
                    review.commit_id
                )?;
            }
            writeln!(
                &mut header,
                "  Current head:       {}",
                pull.head.sha
            )?;
            writeln!(
                &mut header,
                "  Target branch:      {}",
                pull.base.ref_
            )?;
            Ok(header)
        }().unwrap();

        let result = || -> Result<_> {
            let refs: Vec<_> = [&pull.head.sha, &pull.base.sha]
                .into_iter()
                .chain(most_recent_review.iter().map(|review| &review.commit_id))
                .map(|sha| Ref::new(sha))
                .collect();
            pr.repository.fetch_missing(&pr.remote, &refs)?;

            let old = if let Some(review) = most_recent_review {
                review.commit_id
            } else {
                pr.repository
                    .merge_base(&Ref::new(&pull.base.sha), &Ref::new(&pull.head.sha))?
                    .name
            };

            Ok(tool::GitDiffModuloBaseArgs {
                base: Some(pull.base.sha),
                old: Some(old),
                new: Some(pull.head.sha),
                options,
            })
        }();
        let Ok(dmb_args) = result else {
            let err = result.err().unwrap();
            self.inner = Some(Inner {
                pr: pr.into_owned(),
                header,
                dmb_args: None,
                pager: Err(err.to_string()),
            });
            return;
        };

        if self.inner.as_ref().is_some_and(|inner| {
            inner.pr == *pr && inner.dmb_args.as_ref().is_some_and(|args| dmb_args == *args)
        }) {
            return;
        }

        self.options = options;

        let header_copy = header.clone();
        let pager = || -> Result<_> {
            let mut pager_source = DiffPagerSource::new();
            pager_source.push_header(header_copy);
            tool::git_diff_modulo_base(&dmb_args, &pr.repository, &mut pager_source)?;
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
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub repository: Repository,
    pub remote: String,
    pub id: u64,
}

#[derive(Debug)]
pub struct Review<'build> {
    pr: GCow<'build, PullRequest>,
    options: Option<&'build mut GitDiffModuloBaseOptions>,
    search: Option<&'build Regex>,
}
impl<'build> Review<'build> {
    pub fn new(pr: impl Into<GCow<'build, PullRequest>>) -> Self {
        Self {
            pr: pr.into(),
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
        Self {
            search,
            ..self
        }
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

        state_outer.update(
            connections,
            self.pr,
            self.options.as_ref().map(|o| **o).unwrap_or_default());

        let state = state_outer.inner.as_mut().unwrap();

        builder.nest().id(state_id).build(|builder| {
            match &mut state.pager {
            Ok((pager_source, pager_state)) => {
                let has_focus = builder.check_group_focus(state_id);

                let mut pager = Pager::new(pager_source);
                if let Some(regex) = self.search {
                    pager = pager.search(Cow::Borrowed(regex));
                }
                let mut pager_result = pager.build_with_state(builder, "pager", pager_state);

                if has_focus {
                    if builder.on_key_press(KeyCode::Char('C')) {
                        if let Some(options) = self.options {
                            options.combined = !options.combined;
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
            },
            Err(err) => {
                if !state.header.is_empty() {
                    add_multiline_label(builder, &state.header);
                }
                add_text_label(builder, Text::raw(format!("Error: {err}")).style(builder.theme().pane_text.error));
                builder.add_slack();
            },
            }
        });
    }
}
