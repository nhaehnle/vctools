// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;

use diff_modulo_base::*;
use directories::ProjectDirs;
use git_review::{connections, github, logview::add_log_view};
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
mod diff_pager;
mod review;

use crate::{
    review::{PullRequest, Review},
};

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
    let mut args = Cli::parse();

    let dirs = ProjectDirs::from("experimental", "nhaehnle", "vctools").unwrap();
    let config: connections::Config = {
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

    let mut connections = connections::Connections::new(
        config,
        args.github_offline,
        Some(dirs.cache_dir().into()),
    );

    //    println!("{:?}", &config);
    //    println!("{}", dirs.config_dir().display());

    let pr = PullRequest {
        repository: Repository::new(&args.path),
        remote: args.remote.clone(),
        id: args.pull,
    };

    tui_logger::init_logger(LevelFilter::Debug)?;
    tui_logger::set_default_level(LevelFilter::Debug);
    debug!("Starting up");
    trace!("test trace");
    info!("test info");
    warn!("test warn");
    error!("test error");

    let mut terminal = vctuik::init()?;

    let mut running = true;
    let mut show_debug_log = false;
    let mut search: Option<regex::Regex> = None;
    let mut error: Option<String> = None;
    let mut command: Option<String> = None;

    terminal.run(|builder| {
        connections.start_frame(None);

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
            Review::new(&pr)
                .maybe_search(search.as_ref())
                .options(&mut args.dmb_options)
                .build(builder, &mut connections);
        });

        if show_debug_log {
            with_section(builder, "Debug Log", |builder| {
                add_log_view(builder);
            });
        }

        connections.end_frame();

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
