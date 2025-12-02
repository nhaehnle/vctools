// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use clap::Parser;

use diff_modulo_base::*;
use log::{debug, error, info, trace, warn, LevelFilter};
use ratatui::{prelude::*, widgets::Block};
use utils::Result;
use vctuik::{
    command,
    event::{Event, KeyCode, KeyEventKind, MouseEventKind},
    label::add_label,
    prelude::*,
    section::with_section,
    signals,
};

use git_forge_tui::{
    get_project_dirs, github,
    gitservice::GitService,
    load_config,
    logview::add_log_view,
    tui::{actions, Inbox, InboxResult, Review},
    ApiRepository, CompletePullRequest,
};

#[derive(Parser, Debug)]
struct Cli {
    /// Do not access the GitHub API.
    #[clap(long)]
    github_offline: bool,

    #[clap(long)]
    log_file: Option<String>,
}

fn do_main() -> Result<()> {
    let args = Cli::parse();
    let mut dmb_options = tool::GitDiffModuloBaseOptions {
        combined: true,
        ..Default::default()
    };

    if std::env::var("RUST_LOG").is_ok() {
        env_logger::builder()
            .format_timestamp(Some(env_logger::TimestampPrecision::Millis))
            .init();
    } else {
        tui_logger::init_logger(LevelFilter::Debug)?;
        tui_logger::set_default_level(LevelFilter::Debug);
        if let Some(log_file) = args.log_file {
            tui_logger::set_log_file(&log_file)?;
        }
    }
    debug!("Starting up");
    trace!("test trace");
    info!("test info");
    warn!("test warn");
    error!("test error");

    let (refresh_signal, refresh_wait) = signals::make_merge_wakeup();

    let mut connections = github::connections::Connections::new(
        load_config("github.toml")?,
        args.github_offline,
        Some(get_project_dirs().cache_dir().into()),
    );

    let mut git_service = GitService::new(
        &load_config("repositories.toml")?,
        connections.hosts(),
        refresh_signal.clone(),
    );

    let mut terminal = vctuik::init()?;
    terminal.add_merge_wakeup(refresh_wait);

    let mut running = true;
    let mut show_debug_log = false;
    let mut search: Option<regex::Regex> = None;
    let mut error: Option<String> = None;
    let mut command: Option<String> = None;

    terminal.run(|builder| {
        debug!("Start Frame");
        connections.start_frame(Some(builder.start_frame() + Duration::from_millis(50)));
        git_service.start_frame(Duration::from_millis(100));

        if command.is_none() {
            if match builder.peek_event() {
                Some(Event::Key(ev)) if ev.kind == KeyEventKind::Press => true,
                Some(Event::Mouse(ev)) if ev.kind != MouseEventKind::Moved => true,
                _ => false,
            } {
                error = None;
            }
        }

        // Clear the window
        let frame_area = builder.frame().area();
        let block = Block::new().style(builder.theme().pane_background);
        builder.frame().render_widget(block, frame_area);

        let mut inbox = with_section(builder, "Inbox", |builder| {
            Inbox::new().build(builder, &mut connections)
        }).unwrap_or({
            InboxResult {
                has_focus: false,
                selection: None,
            }
        });

        with_section(builder, "Notification", |builder| {
            let Some((host, thread)) = inbox.selection.clone() else {
                add_label(builder, "(no notification selected)");
                builder.add_slack();
                return;
            };

            let url = thread.subject.url.as_ref().map(String::as_str).unwrap_or("<unknown>");
            let id = thread.pull_number();
            if id.is_none() {
                add_label(builder, format!("Notification: {}", url));
                add_label(builder, "(unsupported)");
                builder.add_slack();
                return;
            }

            let api_repo =
                ApiRepository::new(host, thread.repository.owner.login, thread.repository.name);
            let pr = match CompletePullRequest::from_api(api_repo, id.unwrap(), &git_service) {
                Err(err) => {
                    add_label(builder, format!("Notification: {}", url));
                    add_label(builder, format!("{}", err));
                    builder.add_slack();
                    return;
                }
                Ok(pr) => pr,
            };
            Review::new(&git_service, &pr)
                .maybe_search(search.as_ref())
                .options(&mut dmb_options)
                .build(builder, &mut connections);
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
        {
            let mark_done = builder.on_key_press(KeyCode::Char('e'));
            let unsubscribe = builder.on_key_press(KeyCode::Char('M'));
            if mark_done || unsubscribe {
                if let Some((host, notification)) = inbox.selection.take() {
                    let (edit, action) = if mark_done {
                        (github::edit::Edit::MarkNotificationDone(notification.id), "mark as done")
                    } else {
                        (github::edit::Edit::Unsubscribe(notification.id), "unsubscribe")
                    };
                    if let Err(err) =
                        connections.client(host)
                            .unwrap()
                            .borrow_mut()
                            .edit(edit) {
                        error = Some(format!("Failed to {}: {}", action, err));
                    }
                    builder.need_refresh();
                } else {
                    error = Some("No notification selected".into());
                }
            }
        }

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

        connections.end_frame(Some(&refresh_signal));
        git_service.end_frame();

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
