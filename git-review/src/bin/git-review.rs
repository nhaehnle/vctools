// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;

use diff_modulo_base::*;
use directories::ProjectDirs;
use git_review::{logview::add_log_view};
use log::{trace, debug, info, warn, error, LevelFilter};
use ratatui::prelude::*;
use reqwest::header;
use serde::Deserialize;
use std::{borrow::Cow, fmt::Write, ops::{Range}};
use utils::{try_forward, Result};
use vctuik::{
    command,
    event::{Event, KeyCode, KeyEventKind, MouseEventKind},
    pager::{self, PagerSource},
    prelude::*,
    section::with_section,
    stringtools::StrScan,
    theme
};

use git_core::{Ref, Repository};

mod github {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    pub struct Branch {
        #[serde(rename = "ref")]
        pub ref_: String,
        pub sha: String,
    }

    #[derive(Deserialize, Debug)]
    pub struct Pull {
        pub head: Branch,
        pub base: Branch,
    }

    #[derive(Deserialize, Debug)]
    pub struct User {
        pub login: String,
    }

    #[derive(Deserialize, Debug)]
    pub struct Review {
        pub user: User,
        pub commit_id: String,
    }
}

#[derive(Deserialize, Debug)]
struct Host {
    host: String,
    api: String,
    user: String,
    token: String,
}

#[derive(Deserialize, Debug)]
struct Config {
    hosts: Vec<Host>,
}

#[derive(Parser, Debug)]
struct Cli {
    remote: String,
    pull: i32,

    #[clap(flatten)]
    dmb_options: tool::GitDiffModuloBaseOptions,

    /// Behave as if run from the given path.
    #[clap(short = 'C', default_value = ".")]
    path: std::path::PathBuf,
}

trait JsonRequest {
    fn send_json<'a, J>(self) -> Result<J>
    where
        J: serde::de::DeserializeOwned;
}
impl JsonRequest for reqwest::blocking::RequestBuilder {
    fn send_json<'a, J>(self) -> Result<J>
    where
        J: serde::de::DeserializeOwned,
    {
        let (client, request) = self.build_split();
        let request = request?;
        let request_clone = request.try_clone();

        try_forward(
            move || -> Result<J> {
                let response = client.execute(request)?;
                if !response.status().is_success() {
                    Err(format!("HTTP error: {}", response.status()))?
                }

                let body = response.text()?;
                match serde_json::from_str(&body) {
                    Ok(json) => Ok(json),
                    Err(err) => Err(format!("Error parsing JSON: {err}\n{body}\n"))?,
                }
            },
            || format!("Error processing request: {request_clone:?}"),
        )
    }
}

#[derive(Debug)]
enum Element {
    ReviewHeader(String),
    Chunk(diff::Chunk),
    Commit(git_core::RangeDiffMatch),
}
impl Element {
    fn num_lines(&self) -> usize {
        match self {
            Element::ReviewHeader(text) => text.lines().count(),
            Element::Chunk(chunk) => match &chunk.contents {
                diff::ChunkContents::FileHeader { .. } => 2,
                _ => 1,
            },
            Element::Commit(_) => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffDisplayMode {
    Unified,
    OnlyOld,
    OnlyNew,
}
impl Default for DiffDisplayMode {
    fn default() -> Self {
        DiffDisplayMode::Unified
    }
}
impl DiffDisplayMode {
    fn toggled(self) -> Self {
        match self {
            DiffDisplayMode::Unified => DiffDisplayMode::OnlyOld,
            DiffDisplayMode::OnlyOld => DiffDisplayMode::OnlyNew,
            DiffDisplayMode::OnlyNew => DiffDisplayMode::Unified,
        }
    }
}

#[derive(Default)]
struct ReviewPagerSource {
    /// Flat list of all elements of the review
    elements: Vec<Element>,

    /// Global (uncollapsed) line number for every element in `elements`
    global_lines: Vec<usize>,

    /// Indices into `elements` of commit headers
    commits: Vec<usize>,

    /// Indices into `elements` of all file headers
    files: Vec<usize>,

    /// Indices into `elements` of all hunk headers
    hunks: Vec<usize>,

    mode: DiffDisplayMode,

    /// Column widths for range diff matches
    rdm_column_widths: git_core::RangeDiffMatchColumnWidths,

    /// Persistent cursors
    cursors: std::cell::RefCell<pager::PersistentCursors<(usize, usize, bool)>>,
}
impl ReviewPagerSource {
    fn new() -> Self {
        Self::default()
    }

    fn num_global_lines(&self) -> usize {
        self.global_lines
            .last()
            .map_or(0, |&l| l + self.elements.last().unwrap().num_lines())
    }

    fn push_header(&mut self, text: String) {
        assert!(self.elements.is_empty());
        self.global_lines.push(self.num_global_lines());
        self.elements.push(Element::ReviewHeader(text));
    }

    fn toggle_mode(&mut self) {
        self.mode = self.mode.toggled();
        todo!()
    }

    fn truncate_to_header(&mut self) {
        self.elements.truncate(1);
        self.global_lines.truncate(1);
        self.commits.clear();
        self.files.clear();
        self.hunks.clear();
        self.rdm_column_widths = git_core::RangeDiffMatchColumnWidths::default();

        let num_lines = self.num_global_lines();
        let end =
            if num_lines > 0 {
                (num_lines - 1, self.get_raw_line(num_lines - 1, 0, usize::MAX).as_ref().graphemes(true).count())
            } else {
                (0, 0)
            };

        self.cursors.borrow_mut().update(|cursor| {
            if cursor.0 >= num_lines {
                *cursor = (end.0, end.1, true);
            }
        });
    }

    /// Find the nearest folding header at or below the given depth.
    ///
    /// If forward is true, find the smallest index strictly greater than the given index.
    /// 
    /// If forward is false, find the largest index less than or equal to the given index.
    /// Returns (header_idx, depth).
    fn find_folding_header(&self, idx: usize, forward: bool, max_depth: usize) -> Option<(usize, usize)> {
        [
            &self.commits,
            &self.files,
            &self.hunks,
        ]
        .into_iter()
        .take(max_depth.saturating_add(1))
        .enumerate()
        .filter_map(|(depth, indices)| {
            let i = indices.partition_point(|&i| i <= idx);
            if forward {
                if i < indices.len() {
                    Some((indices[i], depth))
                } else {
                    Some((self.elements.len(), 0))
                }
            } else {
                if i == 0 {
                    None
                } else {
                    Some((indices[i - 1], depth))
                }
            }
        })
        .max_by(|a, b| {
            let o = a.0.cmp(&b.0);
            if forward { o.reverse() } else { o }
        })
    }
}
impl diff::ChunkWriter for ReviewPagerSource {
    fn push_chunk(&mut self, chunk: diff::Chunk) {
        self.global_lines.push(self.num_global_lines());

        if matches!(chunk.contents, diff::ChunkContents::FileHeader { .. }) {
            self.files.push(self.elements.len());
        } else if matches!(chunk.contents, diff::ChunkContents::HunkHeader { .. }) {
            self.hunks.push(self.elements.len());
        }

        self.elements.push(Element::Chunk(chunk));
    }
}
impl git_core::RangeDiffWriter for ReviewPagerSource {
    fn push_range_diff_match(&mut self, rdm: git_core::RangeDiffMatch) {
        self.rdm_column_widths = self.rdm_column_widths.max(rdm.column_widths());

        self.global_lines.push(self.num_global_lines());
        self.commits.push(self.elements.len());
        self.elements.push(Element::Commit(rdm));
    }
}
impl PagerSource for ReviewPagerSource {
    fn num_lines(&self) -> usize {
        self.num_global_lines()
    }

    fn get_line(&self, theme: &theme::Text, line: usize, col_no: usize, max_cols: usize) -> Line {
        let idx = self.global_lines.partition_point(|&l| l <= line) - 1;
        let line = line - self.global_lines[idx];

        let (text, style) = match &self.elements[idx] {
            Element::ReviewHeader(text) => (text.clone(), theme.highlight),
            Element::Chunk(chunk) => {
                let style = match &chunk.contents {
                    diff::ChunkContents::FileHeader { .. } => theme.header1,
                    diff::ChunkContents::HunkHeader { .. } => theme.header2,
                    diff::ChunkContents::Line { line } => match line.status {
                        diff::HunkLineStatus::Unchanged => theme.normal,
                        diff::HunkLineStatus::Old(_) => theme.removed,
                        diff::HunkLineStatus::New(_) => theme.added,
                    },
                };

                let mut text = Vec::new();
                chunk.render_text(&mut text);

                (String::from_utf8_lossy(&text).into(), style)
            }
            Element::Commit(rdm) => (rdm.format(self.rdm_column_widths), theme.header0),
        };

        let offset = text
            .row_col_scan((0, 0))
            .find_map(|((l, c), offset)| {
                if l > line {
                    Some(None)
                } else if l == line && c >= col_no {
                    Some(Some(offset))
                } else {
                    None
                }
            })
            .unwrap_or(None);
        let Some(offset) = offset else {
            return Line::default();
        };

        let text = text[offset..].get_first_line(max_cols);
        Line::from(Span::styled(text.to_owned(), style))
    }

    fn get_folding_range(&self, line: usize, parent: bool) -> Option<(Range<usize>, usize)> {
        let idx = self.global_lines.partition_point(|&l| l <= line) - 1;
        let line = line - self.global_lines[idx];

        let (mut header_idx, mut depth) = self.find_folding_header(idx, false, usize::MAX)?;
        if parent && header_idx == idx && line == 0 {
            if idx == 0 || depth == 0{
                return None;
            }
            (header_idx, depth) = self.find_folding_header(idx, false, depth - 1)?;
        }

        let end_idx = self.find_folding_header(header_idx, true, depth).unwrap().0;
        let end_line =
            if end_idx < self.global_lines.len() {
                self.global_lines[end_idx]
            } else {
                self.num_global_lines()
            };

        Some((self.global_lines[header_idx]..end_line, depth))
    }

    fn persist_cursor(
        &self,
        line: usize,
        col: usize,
        _gravity: pager::Gravity,
    ) -> pager::PersistentCursor {
        self.cursors.borrow_mut().add((line, col, false))
    }

    fn retrieve_cursor(&self, cursor: pager::PersistentCursor) -> ((usize, usize), bool) {
        let (line, col, removed) = self.cursors.borrow_mut().take(cursor);
        ((line, col), removed)
    }
}

fn do_main() -> Result<()> {
    let args = Cli::parse();

    let dirs = ProjectDirs::from("experimental", "nhaehnle", "vctools").unwrap();
    let config: Config = {
        let mut config = dirs.config_dir().to_path_buf();
        config.push("github.toml");
        try_forward(
            || {
                Ok(toml::from_str(&String::from_utf8(utils::read_bytes(
                    config,
                )?)?)?)
            },
            || "Error parsing configuration",
        )?
    };

    //    println!("{:?}", &config);
    //    println!("{}", dirs.config_dir().display());

    let git_repo = Repository::new(&args.path);
    let url = git_repo.get_url(&args.remote)?;
    let Some(hostname) = url.hostname() else {
        Err("remote is local")?
    };
    let Some((organization, gh_repo)) = url.github_path() else {
        Err(format!("cannot parse {url} as a GitHub repository"))?
    };

    let Some(host) = config.hosts.iter().find(|host| host.host == hostname) else {
        print!("Host {hostname} not found in config");
        Err("host not configured")?
    };

    let mut default_headers = header::HeaderMap::new();
    default_headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {}", host.token).parse()?,
    );
    default_headers.insert(header::ACCEPT, "application/vnd.github+json".parse()?);
    default_headers.insert("X-GitHub-Api-Version", "2022-11-28".parse()?);

    let client = reqwest::blocking::Client::builder()
        .user_agent("git-review")
        .default_headers(default_headers)
        .build()?;

    let url_api = reqwest::Url::parse(&host.api)?;
    let url_api_repo = url_api.join(format!("repos/{organization}/{gh_repo}/").as_str())?;

    let url_api_pull = url_api_repo.join(format!("pulls/{}", args.pull).as_str())?;
    let url_api_reviews = url_api_repo.join(format!("pulls/{}/reviews", args.pull).as_str())?;

    let pull: github::Pull = client.get(url_api_pull).send_json()?;
    let reviews: Vec<github::Review> = client.get(url_api_reviews).send_json()?;

    let most_recent_review = reviews
        .into_iter()
        .rev()
        .find(|review| review.user.login == host.user);

    let mut review_header = String::new();
    writeln!(
        &mut review_header,
        "Review {}/{}#{}",
        organization, gh_repo, args.pull
    )?;
    if let Some(review) = &most_recent_review {
        writeln!(
            &mut review_header,
            "  Most recent review: {}",
            review.commit_id
        )?;
    }
    writeln!(
        &mut review_header,
        "  Current head:       {}",
        pull.head.sha
    )?;
    writeln!(
        &mut review_header,
        "  Target branch:      {}",
        pull.base.ref_
    )?;

    print!("{review_header}");

    let refs: Vec<_> = [&pull.head.sha, &pull.base.sha]
        .into_iter()
        .chain(most_recent_review.iter().map(|review| &review.commit_id))
        .map(|sha| Ref::new(sha))
        .collect();
    git_repo.fetch_missing(&args.remote, &refs)?;

    let old = if let Some(review) = most_recent_review {
        review.commit_id
    } else {
        git_repo
            .merge_base(&Ref::new(&pull.base.sha), &Ref::new(&pull.head.sha))?
            .name
    };

    let mut dmb_args = tool::GitDiffModuloBaseArgs {
        base: Some(pull.base.sha),
        old: Some(old),
        new: Some(pull.head.sha),
        options: args.dmb_options,
    };

    let mut pager_source = ReviewPagerSource::new();
    pager_source.push_header(review_header);

    tool::git_diff_modulo_base(&dmb_args, &git_repo, &mut pager_source)?;

    let mut terminal = vctuik::init()?;

    tui_logger::init_logger(LevelFilter::Debug)?;
    tui_logger::set_default_level(LevelFilter::Debug);
    debug!("Starting up");
    trace!("test trace");
    info!("test info");
    warn!("test warn");
    error!("test error");

    let mut running = true;
    let mut show_debug_log = false;
    let mut pager_state = pager::PagerState::default();
    let mut search: Option<regex::Regex> = None;
    let mut error: Option<String> = None;
    let mut command: Option<String> = None;

    while running {
        let mut result = Ok(());
        terminal.run_frame(|builder| {
            result = || -> Result<()> {
                if command.is_none() {
                    if match builder.peek_event() {
                        Some(Event::Key(ev)) if ev.kind == KeyEventKind::Press => true,
                        Some(Event::Mouse(ev)) if ev.kind != MouseEventKind::Moved => true,
                        _ => false,
                    } {
                        error = None;
                    }
                }

                let mut pager_result =
                    with_section(builder, "Review", |builder| {
                        let mut pager = pager::Pager::new(&pager_source);
                        if let Some(regex) = &search {
                            pager = pager.search(Cow::Borrowed(regex));
                        }
                        pager.build_with_state(builder, "pager", &mut pager_state)
                    });

                if show_debug_log {
                    with_section(builder, "Debug Log", |builder| {
                        add_log_view(builder);
                    });
                }

                let was_search = command.as_ref().is_some_and(|cmd| cmd.starts_with('/'));

                let action = command::CommandLine::new("command", &mut command)
                    .help("/ to search, q to quit")
                    .build(builder, |builder, _| {
                        if let Some(error) = &error {
                            let area = builder.take_lines_fixed(1);
                            let span = Span::from(error).style(builder.theme().text(builder.theme_context()).error);
                            builder.frame().render_widget(span, area);
                        }
                    });
                match action {
                command::CommandAction::None => {},
                command::CommandAction::Command(cmd) => {
                    error = None;
                    if was_search {
                        if let Some((pattern, pager_result)) = search.as_ref().zip(pager_result.as_mut()) {
                            pager_result.search(pattern, true);
                        }
                    } else if let Some(cmd) = cmd.strip_prefix(':') {
                        if cmd == "log" {
                            show_debug_log = !show_debug_log;
                        } else if cmd == "q" || cmd == "quit" {
                            running = false;
                        } else {
                            error = Some(format!("Unknown command: {cmd}"));
                        }
                    }
                    builder.need_refresh();
                },
                command::CommandAction::Changed(cmd) => {
                    assert!(!cmd.is_empty());

                    error = None;
                    if cmd.starts_with('/') {
                        search = None;
                        if cmd.len() > 1 {
                            match regex::Regex::new(&cmd[1..]) {
                                Ok(regex) => {
                                    search = Some(regex);
                                },
                                Err(e) => {
                                    error = Some(format!("{}", e));
                                }
                            }
                        }
                    } else if cmd.starts_with(':') {
                        // nothing to do
                    } else {
                        error = Some(format!("Unknown command prefix: {}", cmd.chars().next().unwrap()));
                    }
                    builder.need_refresh();
                },
                command::CommandAction::Cancelled => {
                    if was_search {
                        search = None;
                    }
                    error = None;
                },
                }

                // Global key bindings that apply when the review pager is visible
                //
                // Unfortunately, the borrow checker doesn't understand `match pager_result`
                // as a complete move/drop that would also end the borrow of pager_source.
                // So, we need to help it along with the explicit std::mem::drop before any
                // manipulation of pager_source.
                enum SourceUpdate {
                    None,
                    Reload,
                    ToggleMode,
                }
                let update =
                    match pager_result.as_mut() {
                        Some(pager_result) => {
                            if builder.on_key_press(KeyCode::Char('C')) {
                                dmb_args.options.combined = !dmb_args.options.combined;
                                pager_result.move_to(0);
                                SourceUpdate::Reload
                            } else if builder.on_key_press(KeyCode::Char('d')) {
                                SourceUpdate::ToggleMode
                            } else {
                                SourceUpdate::None
                            }
                        },
                        None => SourceUpdate::None,
                    };
                std::mem::drop(pager_result);

                match update {
                    SourceUpdate::None => {},
                    SourceUpdate::Reload => {
                        pager_source.truncate_to_header();
                        tool::git_diff_modulo_base(&dmb_args, &git_repo, &mut pager_source)?;
                        builder.need_refresh();
                    },
                    SourceUpdate::ToggleMode => {
                        pager_source.toggle_mode();
                        builder.need_refresh();
                    },
                }

                // Global key bindings
                if builder.on_key_press(KeyCode::Char('/')) {
                    command = Some("/".into());
                    search = None;
                    builder.need_refresh();
                } else if builder.on_key_press(KeyCode::Char(':')) {
                    command = Some(":".into());
                    builder.need_refresh();
                } else if builder.on_key_press(KeyCode::Char('q')) {
                    running = false;
                }

                Ok(())
            }();

        })?;
    }

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
