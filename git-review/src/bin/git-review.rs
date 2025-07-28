// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;

use diff_modulo_base::*;
use directories::ProjectDirs;
use git_review::{pager, command, stringtools::StrScan};
use ratatui::prelude::*;
use reqwest::header;
use serde::Deserialize;
use std::fmt::Write;
use utils::{try_forward, Result};
use vctuik::{
    self,
    event::KeyCode,
    theme,
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
                diff::DiffChunkContents::FileHeader { .. } => 2,
                _ => 1,
            },
            Element::Commit(_) => 1,
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

    /// Column widths for range diff matches
    rdm_column_widths: git_core::RangeDiffMatchColumnWidths,

    /// Persistent cursors
    cursors: std::cell::RefCell<pager::PersistentCursors<(usize, usize)>>,
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
        self.global_lines.push(self.num_global_lines());
        self.elements.push(Element::ReviewHeader(text));
    }
}
impl diff::ChunkWriter for ReviewPagerSource {
    fn push_chunk(&mut self, chunk: diff::Chunk) {
        self.global_lines.push(self.num_global_lines());

        if matches!(chunk.contents, diff::DiffChunkContents::FileHeader { .. }) {
            self.files.push(self.elements.len());
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
impl pager::PagerSource for ReviewPagerSource {
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
                    diff::DiffChunkContents::FileHeader { .. } => theme.header1,
                    diff::DiffChunkContents::HunkHeader { .. } => theme.header2,
                    diff::DiffChunkContents::Line { line } => match line.status {
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

    fn persist_cursor(
        &self,
        line: usize,
        col: usize,
        _gravity: pager::Gravity,
    ) -> pager::PersistentCursor {
        self.cursors.borrow_mut().add((line, col))
    }

    fn retrieve_cursor(&self, cursor: pager::PersistentCursor) -> ((usize, usize), bool) {
        (self.cursors.borrow_mut().take(cursor), false)
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

    let mut pager_source = ReviewPagerSource::new();
    pager_source.push_header(review_header);

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

    let dmb_args = tool::GitDiffModuloBaseArgs {
        base: Some(pull.base.sha),
        old: Some(old),
        new: Some(pull.head.sha),
        options: args.dmb_options,
    };

    tool::git_diff_modulo_base(dmb_args, git_repo, &mut pager_source)?;

    let mut terminal = vctuik::init()?;

    let mut running = true;
    let mut command: Option<String> = None;

    while running {
        terminal.run_frame(|builder| {
            pager::Pager::new(&pager_source).build(builder, "pager");

            command::CommandLine::new("command", &mut command)
                .help("q to quit")
                .build(builder);

            if builder.on_key_press(KeyCode::Char('q')) {
                running = false;
                return;
            }
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
