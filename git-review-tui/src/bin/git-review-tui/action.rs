
use std::rc::Rc;

use ratatui::{
    buffer::Buffer,
    crossterm::event::{Event, KeyCode, KeyEventKind},
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{block::BorderType, Block, Borders, Clear, Paragraph, StatefulWidget, Widget},
};

use tui_input::{backend::crossterm::EventHandler, Input};

use crate::theme::{Theme, Themed};

#[derive(Debug)]
struct Command {
    name: String,
    titles: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Commands {
    commands: Vec<Command>,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandId(usize);

pub struct CommandsMap<V> {
    commands: Vec<Option<V>>,
}
impl<V> CommandsMap<V> {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
        }
    }

    pub fn set(&mut self, id: CommandId, value: V) {
        self.commands.resize_with(id.0 + 1, Default::default);
        self.commands[id.0] = Some(value);
    }

    pub fn get(&self, id: CommandId) -> Option<&V> {
        self.commands.get(id.0).and_then(Option::as_ref)
    }
}
impl<V> std::fmt::Debug for CommandsMap<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CommandsMap {{ ... }}")
    }
}

#[derive(Debug)]
struct FilteredCommand {
    id: CommandId,
    title_idx: usize,
    matching_chars: Vec<usize>,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    fn get(&self, id: CommandId) -> &Command {
        &self.commands[id.0]
    }

    pub fn add_command<N, T>(&mut self, name: N, titles: &[T]) -> CommandId
    where
        N: Into<String>,
        T: Copy + Into<String>,
    {
        self.commands.push(Command {
            name: name.into(),
            titles: titles.iter().map(|t| (*t).into()).collect(),
        });

        CommandId(self.commands.len() - 1)
    }

    fn filter_eval(&self, id: CommandId, query: &str) -> Option<(usize, FilteredCommand)> {
        let command = self.get(id);
        command.titles.iter().enumerate().map(|(title_idx, title)| {
            let title = title.to_lowercase();

            let mut matching_chars = Vec::new();

            let mut tchars = title.char_indices().peekable();
            let mut cost: Option<usize> = Some(0);
            let mut in_word = false;

            for qch in query.chars() {
                if qch.is_whitespace() {
                    in_word = false;
                }

                let mut skipped = false;
                while let Some((_, tch)) = tchars.peek() {
                    if *tch == qch { break; }
                    skipped = true;
                    tchars.next();
                }

                match tchars.next() {
                None => {
                    cost = None;
                    break
                },
                Some((idx, _)) => {
                    if skipped && in_word {
                        *cost.as_mut().unwrap() += 1;
                    }
                    matching_chars.push(idx);
                },
                }

                if !qch.is_whitespace() {
                    in_word = true;
                }
            }

            cost.map(|cost| (
                10 * cost + title_idx,
                FilteredCommand {
                    id,
                    title_idx,
                    matching_chars,
                },
            ))
        }).filter_map(|x| x).min_by_key(|(cost, _)| *cost)
    }

    fn filter(&self, query: &str) -> Vec<FilteredCommand> {
        if query.is_empty() {
            return (0..self.commands.len()).into_iter()
                .map(|i| FilteredCommand {
                    id: CommandId(i),
                    title_idx: 0,
                    matching_chars: Vec::new(),
                })
                .collect();
        }

        let query = query.to_lowercase();

        let mut filtered: Vec<_> =
            (0..self.commands.len()).into_iter()
                .filter_map(|i| {
                    self.filter_eval(CommandId(i), &query)
                })
            .collect();

        filtered.sort_by_key(|(cost, _)| *cost);

        filtered.into_iter().map(|(_, filtered)| filtered).collect()
    }
}

#[derive(Debug)]
struct FilterState {
    filtered: Vec<FilteredCommand>,
    selected: usize,
    scroll: usize,
}
impl FilterState {
    fn new(commands: &Commands) -> Self {
        Self {
            filtered: commands.filter(""),
            selected: 0,
            scroll: 0,
        }
    }

    fn get_selected(&self) -> Option<CommandId> {
        self.filtered.get(self.selected).map(|filtered| filtered.id)
    }

    fn set_query(&mut self, commands: &Commands, query: &str) {
        self.filtered = commands.filter(query);
        self.selected = 0;
        self.scroll = 0;
    }

    fn handle_up(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    fn handle_down(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.scroll = std::cmp::min(self.scroll, self.selected);
        }
    }

    fn stylize_match<'text>(
            text: &'text str,
            matching_chars: &[usize],
            normal_style: Style,
            match_style: Style) -> Vec<Span<'text>> {
        let mut spans = Vec::new();

        let mut match_begin = None;
        let mut ack = 0;

        for idx in matching_chars.iter() {
            if ack != *idx {
                if let Some(match_begin) = match_begin.take() {
                    spans.push(Span::styled(&text[match_begin..ack], match_style));
                }
                spans.push(Span::styled(&text[ack..*idx], normal_style));
            }

            match_begin.get_or_insert(*idx);
            ack = idx + 1;
            while !text.is_char_boundary(ack) {
                ack += 1;
            }
        }

        if let Some(match_begin) = match_begin.take() {
            spans.push(Span::styled(&text[match_begin..ack], match_style));
        }
        if ack != text.len() {
            spans.push(Span::styled(&text[ack..text.len()], normal_style));
        }

        spans
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, theme: Option<&Theme>, commands: &Commands) {
        if area.y < 10 {
            return;
        }

        let max_height = std::cmp::min(area.y, 5 + (std::cmp::max(5, area.y) - 5) / 4) as usize;
        let content_lines = std::cmp::max(1, self.filtered.len());
        let content_height = std::cmp::min(content_lines, max_height - 1);

        let popup_height = content_height as u16 + 1;
        let popup_area = Rect::new(area.x, area.y - popup_height, area.width, popup_height);
        let content_area = Rect::new(area.x, popup_area.y + 1, area.width, content_height as u16);

        Clear.render(popup_area, buf);

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + content_height {
            self.scroll = self.selected - content_height + 1;
        }

        let text = if self.filtered.is_empty() {
            Text::from("(no matching command)")
        } else {
            let normal_style = theme.map_or(Style::default(), |theme| theme.modal_content);
            let match_style = theme.map_or(Style::default(), |theme| theme.modal_highlight);

            let normal_selected_style = theme.map_or(Style::default(), |theme| theme.modal_selection);
            let match_selected_style = normal_selected_style.patch(match_style);

            let mut lines: Vec<_> =
                self.filtered.iter()
                .enumerate()
                .skip(self.scroll)
                .take(content_height)
                .map(|(i, filtered)| {
                    let command = commands.get(filtered.id);

                    let (normal_style, match_style) =
                    if i == self.selected {
                        (normal_selected_style, match_selected_style)
                    } else {
                        (normal_style, match_style)
                    };

                    let mut spans;
                    if filtered.title_idx == 0 {
                        spans = FilterState::stylize_match(
                            &command.titles[0],
                            &filtered.matching_chars,
                            normal_style,
                            match_style);
                    } else {
                        spans = vec![
                            Span::styled(&command.titles[0], normal_style),
                            Span::styled(" (aka ", normal_style),
                        ];
                        spans.append(&mut FilterState::stylize_match(
                            &command.titles[filtered.title_idx],
                            &filtered.matching_chars,
                            normal_style,
                            match_style));
                        spans.push(Span::styled(")", normal_style));
                    }

                    let mut line = Line::from(spans);
                    if i == self.selected {
                        line = line.on_yellow();
                    }
                    line
                })
                .collect();
            lines.reverse();
            Text::from(lines)
        };

        Block::new()
            .border_type(BorderType::Double)
            .borders(Borders::TOP)
            .opt_theme_modal_pane(theme)
            .render(popup_area, buf);
        Paragraph::new(text).render(content_area, buf);
    }
}
impl Default for FilterState {
    fn default() -> Self {
        Self {
            filtered: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionBarMode {
    Command,
    Search,
}

pub enum Response {
    None,
    Cancel,
    Command(CommandId),
}

#[derive(Debug)]
enum ActionBarStateImpl {
    Idle,
    Active {
        mode: ActionBarMode,
        input: Input,
        filter: FilterState,
    },
}

#[derive(Debug)]
pub struct ActionBarState {
    state: ActionBarStateImpl,
}

impl ActionBarState {
    pub fn new() -> Self {
        Self {
            state: ActionBarStateImpl::Idle,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, ActionBarStateImpl::Active { .. })
    }

    pub fn activate(&mut self, mode: ActionBarMode, commands: &Commands) {
        let filter = match mode {
            ActionBarMode::Command => FilterState::new(&commands),
            ActionBarMode::Search => FilterState::default(),
        };

        self.state = ActionBarStateImpl::Active {
            mode,
            input: Input::new((match mode {
                ActionBarMode::Command => ":",
                ActionBarMode::Search => "/",
            }).into()),
            filter,
        };
    }

    pub fn handle_event(&mut self, ev: Event, commands: &Commands) -> Response {
        let ActionBarStateImpl::Active { mode, input, filter } = &mut self.state else { return Response::Cancel };

        match ev {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match key.code {
                    KeyCode::Enter => {
                        let response =
                            if *mode == ActionBarMode::Command {
                                match filter.get_selected() {
                                    Some(id) => Response::Command(id),
                                    None => Response::Cancel,
                                }
                            } else {
                                Response::Cancel
                            };
                        self.state = ActionBarStateImpl::Idle;
                        response
                    }
                    KeyCode::Esc => {
                        self.state = ActionBarStateImpl::Idle;
                        Response::Cancel
                    }
                    KeyCode::Up => {
                        filter.handle_up();
                        Response::None
                    },
                    KeyCode::Down => {
                        filter.handle_down();
                        Response::None
                    },
                    _ => {
                        if let Some(change) = input.handle_event(&ev) {
                            if change.value && input.value().is_empty() {
                                self.state = ActionBarStateImpl::Idle;
                                Response::Cancel
                            } else {
                                if change.value && *mode == ActionBarMode::Command {
                                    filter.set_query(&commands, &input.value()[1..]);
                                }
                                Response::None
                            }
                        } else {
                            Response::None
                        }
                    }
                }
            }
            _ => { Response::None }
        }
    }
}

#[derive(Debug)]
pub struct ActionBar<'data, 'theme> {
    commands: &'data Commands,
    theme: Option<&'theme Theme>,
}

impl<'data, 'theme> ActionBar<'data, 'theme> {
    pub fn new(commands: &'data Commands) -> Self {
        Self {
            commands,
            theme: None,
        }
    }

    pub fn theme(mut self, theme: &'theme Theme) -> Self {
        self.theme = Some(theme);
        self
    }
}
impl<'data, 'theme> StatefulWidget for &ActionBar<'data, 'theme> {
    type State = ActionBarState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut ActionBarState) {
        match &mut state.state {
            ActionBarStateImpl::Idle => {
                Paragraph::new("Press ':' to enter command mode, '/' to enter search mode")
                    .set_style_opt(self.theme.map(|theme| theme.status_bar))
                    .render(area, buf);
            }
            ActionBarStateImpl::Active { mode, input, filter } => {
                let scroll = input.visual_scroll(area.width as usize);
                Paragraph::new(input.value())
                    .opt_theme_status_bar(self.theme)
                    .scroll((0, scroll as u16))
                    .render(area, buf);
                if *mode == ActionBarMode::Command {
                    filter.render(area, buf, self.theme, &self.commands);
                }
            }
        }
    }
}
