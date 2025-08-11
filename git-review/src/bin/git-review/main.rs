// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;

use diff_modulo_base::*;
use directories::ProjectDirs;
use git_review::{github, logview::add_log_view};
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

mod actions;
mod review;

use crate::{
    review::{Review, ReviewState},
};

#[derive(Deserialize, Debug)]
struct Config {
    hosts: Vec<github::Host>,
}

#[derive(Parser, Debug)]
struct Cli {
    remote: String,
    pull: u64,

    #[clap(flatten)]
    dmb_options: tool::GitDiffModuloBaseOptions,

    /// Behave as if run from the given path.
    #[clap(short = 'C', default_value = ".")]
    path: std::path::PathBuf,

    /// Do not access the GitHub API.
    #[clap(long)]
    github_offline: bool,
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

    tui_logger::init_logger(LevelFilter::Debug)?;
    tui_logger::set_default_level(LevelFilter::Debug);
    debug!("Starting up");
    trace!("test trace");
    info!("test info");
    warn!("test warn");
    error!("test error");

    let mut client =
        github::Client::build(host.clone())
            .offline(args.github_offline)
            .cache_dir(dirs.cache_dir().join(&host.host))
            .new()?;

    let client_frame = client.frame(None);
    let pull = client_frame.pull(organization, gh_repo, args.pull).ok()?;
    let reviews = client_frame.reviews(organization, gh_repo, args.pull).ok()?;

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

    let dmb_args = tool::GitDiffModuloBaseArgs {
        base: Some(pull.base.sha),
        old: Some(old),
        new: Some(pull.head.sha),
        options: args.dmb_options,
    };

    let mut review = ReviewState::new(review_header, dmb_args, git_repo)?;

    let mut terminal = vctuik::init()?;

    let mut running = true;
    let mut show_debug_log = false;
    let mut search: Option<regex::Regex> = None;
    let mut error: Option<String> = None;
    let mut command: Option<String> = None;

    terminal.run(|builder| {
        if command.is_none() {
            if match builder.peek_event() {
                Some(Event::Key(ev)) if ev.kind == KeyEventKind::Press => true,
                Some(Event::Mouse(ev)) if ev.kind != MouseEventKind::Moved => true,
                _ => false,
            } {
                error = None;
            }
        }

        with_section(builder, "Review", |builder| {
            let widget = Review::new().maybe_search(search.as_ref());
            if let Err(err) = widget.build(builder, &mut review) {
                error!("Review: {err}");
                error = Some(format!("{err}"));
            }
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
                if let Some(pattern) = search.as_ref() {
                    builder.inject_custom(actions::Search(pattern.clone()));
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

        Ok(running)
    })?;

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
