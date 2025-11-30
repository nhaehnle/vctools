// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use clap::Parser;

use diff_modulo_base::*;
use log::{debug, error, info, trace, warn, LevelFilter};
use ratatui::prelude::*;
use vctuik::{
    command,
    event::{Event, KeyCode, KeyEventKind, MouseEventKind},
    prelude::*,
    section::with_section,
    signals,
};

use git_forge_tui::{
    get_project_dirs, github, gitservice, load_config,
    logview::add_log_view,
    tui::{actions, Review},
    CompletePullRequest, GitRepository,
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

    let mut connections = github::connections::Connections::new(
        load_config("github.toml")?,
        args.github_offline,
        Some(get_project_dirs().cache_dir().into()),
    );

    //    println!("{:?}", &config);
    //    println!("{}", dirs.config_dir().display());

    let (refresh_signal, refresh_wait) = signals::make_merge_wakeup();
    let git_repository = GitRepository::new(args.path, args.remote);
    let mut git_service = gitservice::GitService::new(
        &gitservice::Config::default(),
        connections.hosts(),
        refresh_signal.clone(),
    );
    let pr = CompletePullRequest::from_git(
        git_repository,
        args.pull,
        connections.hosts(),
        &git_core::SimpleExecutionProvider,
    )?;

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

    terminal.add_merge_wakeup(refresh_wait);

    terminal.run(|builder| {
        connections.start_frame(Some(builder.start_frame() + Duration::from_millis(150)));
        git_service.start_frame(Duration::from_millis(150));

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
            Review::new(&git_service, &pr)
                .maybe_search(search.as_ref())
                .options(&mut args.dmb_options)
                .build(builder, &mut connections);
        });

        if show_debug_log {
            with_section(builder, "Debug Log", |builder| {
                add_log_view(builder);
            });
        }

        connections.end_frame(Some(&refresh_signal));
        git_service.end_frame();

        let was_search = command.as_ref().is_some_and(|cmd| cmd.starts_with('/'));

        let action = command::CommandLine::new("command", &mut command)
            .help("/ to search, q to quit")
            .build(builder, |builder, _| {
                if let Some(error) = &error {
                    let area = builder.take_lines_fixed(1);
                    let span = Span::from(error)
                        .style(builder.theme().text(builder.theme_context()).error);
                    builder.frame().render_widget(span, area);
                }
            });
        match action {
            command::CommandAction::None => {}
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
            }
            command::CommandAction::Changed(cmd) => {
                assert!(!cmd.is_empty());

                error = None;
                if cmd.starts_with('/') {
                    search = None;
                    if cmd.len() > 1 {
                        match regex::Regex::new(&cmd[1..]) {
                            Ok(regex) => {
                                search = Some(regex);
                            }
                            Err(e) => {
                                error = Some(format!("{}", e));
                            }
                        }
                    }
                } else if cmd.starts_with(':') {
                    // nothing to do
                } else {
                    error = Some(format!(
                        "Unknown command prefix: {}",
                        cmd.chars().next().unwrap()
                    ));
                }
                builder.need_refresh();
            }
            command::CommandAction::Cancelled => {
                if was_search {
                    search = None;
                }
                error = None;
            }
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
