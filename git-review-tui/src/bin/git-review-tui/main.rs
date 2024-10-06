use std::{
    cell::RefCell,
    hash::Hash,
    io::{self, BufReader},
    fs::File,
    rc::Rc
};
use tui_logger::{TuiLoggerSmartWidget, TuiLoggerWidget, TuiWidgetEvent, TuiWidgetState};
use vctools_utils::preamble::*;

use log::{trace, debug, info, warn, error, LevelFilter};

use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    widgets::{
        Block, Borders, BorderType, Paragraph, StatefulWidget, Widget
    },
    DefaultTerminal
};
use directories::ProjectDirs;
use tui_tree_widget::{Tree, TreeItem, TreeState};
use serde::Deserialize;

mod action;
mod github;
mod model;
mod msgbox;
mod panes;
mod topwidget;

use action::{ActionBar, ActionBarMode, ActionBarState, Commands, CommandsMap};
use github::{GitHubAccount};
use msgbox::MessageBox;
use panes::{PanesState, PanesLayout, Pane, Panes};
use topwidget::TopWidget;

const PANE_THREADS: usize = 0;
const PANE_LOGGING: usize = 1;

#[derive(Debug, Deserialize)]
struct Account {
    name: String,
    kind: String,

    #[serde(flatten)]
    github: GitHubAccount,
}

#[derive(Debug, Default, Deserialize)]
struct Settings {
    accounts: Vec<Account>,
}

struct AppState {
    exit: bool,
    action_bar: ActionBarState,
    terminal: Option<Rc<RefCell<DefaultTerminal>>>,
    accounts: TreeState<usize>,
    logging: TuiWidgetState,
    panes: PanesState,
}

struct App {
    settings: Settings,
    project_dirs: ProjectDirs,
    commands: Rc<Commands>,
    commands_map: CommandsMap<for<'a, 'b> fn(&mut App)>,
    action_bar: ActionBar,
    state: AppState,
    accounts: Vec<TreeItem<'static, usize>>,
}

impl App {
    pub fn init() -> io::Result<Self> {
        let project_dirs = ProjectDirs::from("", "", "git-review-tui").unwrap();

        std::fs::create_dir_all(&project_dirs.config_dir())?;
        std::fs::create_dir_all(&project_dirs.cache_dir())?;

        let mut commands = Commands::new();
        let mut commands_map: CommandsMap<for<'a, 'b> fn(&mut App)> = CommandsMap::new();

        commands_map.set(commands.add_command("quit", &["Quit", "Exit"]), App::cmd_quit);
        commands_map.set(commands.add_command("account-add", &["Add Account"]), App::cmd_account_add);
        commands_map.set(
            commands.add_command("toggle-debug-log", &["Toggle Debug Log"]),
            App::cmd_toggle_debug_log);

        commands.add_command("foo", &["Foo"]);
        commands.add_command("bar", &["Bar"]);
        commands.add_command("baz", &["Baz"]);
        commands.add_command("abiba", &["Abiba"]);

        let commands = Rc::new(commands);
        let action_bar = ActionBar::new(commands.clone());

        let mut panes_state = PanesState::default();
        panes_state.set_visible(PANE_LOGGING, false);

        Ok(Self {
            settings: Settings::default(),
            project_dirs,
            commands,
            commands_map,
            action_bar,
            accounts: Vec::new(),
            state: AppState {
                exit: false,
                action_bar: ActionBarState::new(),
                terminal: None,
                accounts: TreeState::default(),
                logging: TuiWidgetState::default(),
                panes: panes_state,
            },
        })
    }

    fn post_init(&mut self) -> io::Result<()> {
        let path = self.project_dirs.config_dir().join("settings.json");
        let result = try_forward(|| {
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(err) => {
                    if err.kind() == io::ErrorKind::NotFound { return Ok(()) }
                    return Err(err.into())
                },
            };

            self.settings = serde_json::from_reader(BufReader::new(file))?;
            Ok(())
        }, || format!("Failed to read settings from '{}'", path.display()).to_string());

        if let Err(err) = result {
            MessageBox::new(self, "Error", &err.to_string()).run()?;
        } else {
            for (idx, account) in self.settings.accounts.iter().enumerate() {
                self.accounts.push(
                    TreeItem::new(
                        idx, account.name.clone(),
                        vec![TreeItem::new_leaf(std::usize::MAX, "Loading...")],
                    )?
                );
            }
        }

        Ok(())
    }

    pub fn run(&mut self, terminal: DefaultTerminal) -> io::Result<()> {
        let terminal = Rc::new(RefCell::new(terminal));
        self.state.terminal = Some(terminal.clone());

        self.post_init()?;

        while !self.state.exit {
            terminal.borrow_mut().draw(|frame| self.render_to_frame(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let ev = event::read()?;

        if self.state.action_bar.is_active() {
            match self.state.action_bar.handle_event(ev, &self.action_bar) {
                action::Response::Command(cmd) => {
                    if let Some(cmd) = self.commands_map.get(cmd) {
                        cmd(self);
                    }
                },
                _ => {},
            }
            return Ok(())
        }

        let mut handled = match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.handle_key_press(key)
            },
            _ => false,
        };

        if !handled {
            handled = match self.state.panes.handle_event(ev) {
                panes::Response::Route(pane, ev) => {
                    match pane {
                        PANE_THREADS => handle_tree_view_event(&mut self.state.accounts, ev),
                        PANE_LOGGING => handle_debug_log_event(&mut self.state.logging, ev),
                        _ => false,
                    }
                },
                panes::Response::Handled => true,
                panes::Response::NotHandled => false,
            };
        }

        Ok(())
    }

    fn handle_key_press(&mut self, key: event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('q') => self.state.exit = true,
            KeyCode::Char(':') => self.state.action_bar.activate(ActionBarMode::Command, &self.action_bar),
            KeyCode::Char('/') => self.state.action_bar.activate(ActionBarMode::Search, &self.action_bar),
            _ => return false,
        }
        true
    }

    fn cmd_quit(&mut self) {
        self.state.exit = true;
    }

    fn cmd_account_add(&mut self) {
        drop(MessageBox::new(self, "Add Account", "Not implemented").run());
    }

    fn cmd_toggle_debug_log(&mut self) {
        self.state.panes.set_visible(PANE_LOGGING, !self.state.panes.is_visible(PANE_LOGGING))
    }
}

impl TopWidget for App {
    fn terminal(&self) -> Rc<RefCell<DefaultTerminal>> {
        self.state.terminal.as_ref().unwrap().clone()
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);

        let mut pane_threads = Pane::new(PANE_THREADS, "Reviews");

        if self.settings.accounts.is_empty() {
            pane_threads = pane_threads.widget(
                Paragraph::new("No accounts configured. Press ':' and select \"Add Account\"")
                    .black()
                    .on_white()
            );
        } else {
            let tree = Tree::new(&self.accounts).unwrap()
                .style(Style::new().black().on_white())
                .highlight_style(Style::new().blue().on_yellow());

            pane_threads = pane_threads.stateful_widget(tree, &mut self.state.accounts);
        }

        let logging = TuiLoggerWidget::default()
            .style(Style::default().black().on_white())
            .state(&self.state.logging);

        Panes::new(vec![
            pane_threads
                .constraint(Constraint::Fill(10)),
            Pane::new(PANE_LOGGING, "Debug Log")
                .widget(logging)
                .constraint(Constraint::Fill(10)),
        ])
        .render(vertical[0], buf, &mut self.state.panes);

        self.action_bar.render(vertical[1], buf, &mut self.state.action_bar);
    }
}

fn handle_tree_view_event<I: Clone + PartialEq + Eq + Hash>(state: &mut TreeState<I>, ev: Event) -> bool {
    match ev {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            match key.code {
                KeyCode::Left => { state.key_left(); },
                KeyCode::Right => { state.key_right(); },
                KeyCode::Down => { state.key_down(); },
                KeyCode::Up => { state.key_up(); },
                KeyCode::Esc => { state.select(Vec::new()); },
                KeyCode::Home => { state.select_first(); },
                KeyCode::End => { state.select_last(); },
                KeyCode::PageDown => { for _ in 0..5 { state.key_down(); } },
                KeyCode::PageUp => { for _ in 0..5 { state.key_up(); } },
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

fn handle_debug_log_event(state: &mut TuiWidgetState, ev: Event) -> bool {
    match ev {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            let widget_event =
                match key.code {
                    KeyCode::Char(' ') => TuiWidgetEvent::SpaceKey,
                    KeyCode::Down => TuiWidgetEvent::DownKey,
                    KeyCode::Up => TuiWidgetEvent::UpKey,
                    KeyCode::Left => TuiWidgetEvent::LeftKey,
                    KeyCode::Right => TuiWidgetEvent::RightKey,
                    KeyCode::Char('+') => TuiWidgetEvent::PlusKey,
                    KeyCode::Char('-') => TuiWidgetEvent::MinusKey,
                    KeyCode::Char('h') => TuiWidgetEvent::HideKey,
                    KeyCode::Char('f') => TuiWidgetEvent::FocusKey,
                    KeyCode::PageDown => TuiWidgetEvent::NextPageKey,
                    KeyCode::PageUp => TuiWidgetEvent::PrevPageKey,
                    _ => return false,
                };
            state.transition(widget_event);
        }
        _ => return false,
    }
    true
}

fn main() -> Result<()> {
    tui_logger::init_logger(LevelFilter::Debug)?;
    tui_logger::set_default_level(LevelFilter::Debug);
    debug!("Starting up");
    trace!("test trace");
    info!("test info");
    warn!("test warn");
    error!("test error");

    let mut app = App::init()?;

    let mut terminal = ratatui::init();
    terminal.clear()?;
    let app_result = app.run(terminal);
    ratatui::restore();

    app_result?;

    Ok(())
}
