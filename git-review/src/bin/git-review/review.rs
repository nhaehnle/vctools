// SPDX-License-Identifier: GPL-3.0-or-later

use std::{borrow::Cow, ops::Range};

use diff_modulo_base::{diff, git_core::{self, Repository}, tool::{self, GitDiffModuloBaseArgs}};
use ratatui::text::{Line, Span};
use regex::Regex;
use vctuik::{
    event::KeyCode, pager::{Pager, PagerState}, prelude::*, state::Builder
};

use crate::{actions, diff_pager::DiffPagerSource};

#[derive(Debug)]
pub struct ReviewState {
    args: GitDiffModuloBaseArgs,
    git_repo: Repository,
    pager_source: DiffPagerSource,
    pager_state: PagerState,
}
impl ReviewState {
    pub fn new(header: String, args: GitDiffModuloBaseArgs, git_repo: Repository) -> Result<Self> {
        let mut pager_source = DiffPagerSource::new();
        pager_source.push_header(header);
        tool::git_diff_modulo_base(&args, &git_repo, &mut pager_source)?;

        Ok(Self {
            args,
            git_repo,
            pager_source,
            pager_state: PagerState::default(),
        })
    }
}

#[derive(Debug)]
pub struct Review<'build> {
    search: Option<&'build Regex>,
}
impl<'build> Review<'build> {
    pub fn new() -> Self {
        Self {
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

    pub fn build(self, builder: &mut Builder, state: &mut ReviewState) -> Result<()> {
        let state_id = builder.add_state_id("review");
        let mut result = Ok(());

        builder.nest().id(state_id).build(|builder| {
            let has_focus = builder.check_group_focus(state_id);

            let mut pager = Pager::new(&state.pager_source);
            if let Some(regex) = self.search {
                pager = pager.search(Cow::Borrowed(regex));
            }
            let mut pager_result = pager.build_with_state(builder, "pager", &mut state.pager_state);

            if has_focus {
                if builder.on_key_press(KeyCode::Char('C')) {
                    state.args.options.combined = !state.args.options.combined;
                    pager_result.move_to(0);
                    std::mem::drop(pager_result);

                    state.pager_source.truncate_to_header();
                    if let Err(err) =
                        tool::git_diff_modulo_base(
                            &state.args,
                            &state.git_repo,
                            &mut state.pager_source) {
                        result = Err(err);
                    }
                    builder.need_refresh();
                } else if builder.on_key_press(KeyCode::Char('d')) {
                    std::mem::drop(pager_result);
                    state.pager_source.toggle_mode();
                    builder.need_refresh();
                } else if let Some(search) = builder.on_custom::<actions::Search>() {
                    pager_result.search(&search.0, true);
                    builder.need_refresh();
                }
            }
        });

        result
    }
}
