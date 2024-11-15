use std::{borrow::Cow, collections::HashMap};

use ratatui::{
    layout::{Position, Rect},
    Frame,
};

use vctuik_unsafe_internals::state;

use crate::{
    event::{Event, KeyEventKind, KeyCode},
    theme::{Context, Theme},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

pub enum Renderable<'render> {
    Span(Rect, ratatui::text::Span<'render>),
    Line(Rect, ratatui::text::Line<'render>),
    Text(Rect, ratatui::text::Text<'render>),
    Block(Rect, ratatui::widgets::Block<'render>),
    SetCursor(Position),
    Other(Box<dyn FnOnce(&mut Frame) + 'render>),
    None,
}
impl Renderable<'_> {
    pub fn render(self, frame: &mut Frame) {
        match self {
            Renderable::Span(area, span) => frame.render_widget(span, area),
            Renderable::Line(area, line) => frame.render_widget(line, area),
            Renderable::Text(area, text) => frame.render_widget(text, area),
            Renderable::Block(area, block) => frame.render_widget(block, area),
            Renderable::SetCursor(position) => frame.set_cursor_position(position),
            Renderable::Other(other) => other(frame),
            Renderable::None => {},
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Focus {
    ghost: Option<String>,
    index: usize,
}

#[derive(Default)]
struct IdState {
    id_map: HashMap<String, WidgetId>,
    ids: Vec<String>,
    focus_chain: Vec<WidgetId>,
    focus: Option<Focus>,
}
impl IdState {
    fn clear(&mut self) {
        self.id_map.clear();
        self.ids.clear();
        self.focus_chain.clear();
        self.focus = None;
    }

    fn can_focus(&self) -> bool {
        !self.focus_chain.is_empty()
    }

    fn move_focus(&mut self, next: bool) {
        assert!(!self.focus_chain.is_empty());

        let index =
            if next {
                self.focus.as_ref()
                    .map(|Focus { ghost, index }| index + if ghost.is_some() { 0 } else { 1 })
                    .filter(|&index| index < self.focus_chain.len())
                    .unwrap_or(0)
            } else {
                self.focus.as_ref()
                    .and_then(|focus| focus.index.checked_sub(1))
                    .unwrap_or(self.focus_chain.len().saturating_sub(1))
            };

        self.focus = Some(Focus { ghost: None, index });
    }
}

#[derive(Default)]
struct IdStore {
    previous: IdState,
    current: IdState,
}

#[derive(Default)]
pub(crate) struct Store {
    ids: IdStore,
    state: state::Store<WidgetId>,
}

pub(crate) type EventHandlers<'handler> = Vec<Box<dyn (FnMut(&Event) -> Handled) + 'handler>>;

pub(crate) struct BuildStore<'render, 'handler> {
    ids: &'handler mut IdStore,
    state_builder: state::Builder<'handler, WidgetId>,
    render: Vec<Renderable<'render>>,
    handlers: EventHandlers<'handler>,
    theme: &'render Theme,
}
impl<'render, 'handler> BuildStore<'render, 'handler> {
    pub(crate) fn new(state: &'handler mut Store, theme: &'render Theme) -> Self {
        let ids = &mut state.ids;
        std::mem::swap(&mut ids.previous, &mut ids.current);
        ids.current.clear();

        let state_builder = state::Builder::new(&mut state.state);

        BuildStore {
            ids,
            state_builder,
            render: Vec::new(),
            handlers: Vec::new(),
            theme,
        }
    }

    pub(crate) fn finish(mut self) -> (Vec<Renderable<'render>>, EventHandlers<'handler>) {
        let previous = &mut self.ids.previous;
        let current = &mut self.ids.current;

        if current.focus.is_none() && !current.focus_chain.is_empty() {
            if let Some(Focus { mut ghost, index }) = previous.focus.take() {
                if ghost.is_none() {
                    ghost = Some(std::mem::take(&mut previous.ids[previous.focus_chain[index].0]));
                }

                let ghost_index =
                    previous.focus_chain[(index + 1)..]
                        .iter()
                        .copied()
                        .filter_map(|old_id| {
                            current.id_map.get(&previous.ids[old_id.0])
                                .map(|&id| id)
                        })
                        .find_map(|new_id| {
                            current.focus_chain.iter()
                                .enumerate()
                                .find_map(|(index, &id)| (id == new_id).then_some(index))
                        })
                        .unwrap_or(0);
                current.focus = Some(Focus { ghost, index: ghost_index });
            }
        }

        if current.can_focus() {
            self.handlers.push(Box::new(|event| {
                match event {
                    Event::Key(ev) if ev.kind == KeyEventKind::Press => {
                        let next = match ev.code {
                            KeyCode::Tab => true,
                            KeyCode::BackTab => false,
                            _ => return Handled::No,
                        };
                        current.move_focus(next);
                        Handled::Yes
                    }
                    _ => Handled::No,
                }
            }));
        }

        (self.render, self.handlers)
    }
}

pub struct Builder<'builder, 'render, 'handler> {
    store: &'builder mut BuildStore<'render, 'handler>,
    id_prefix: String,
    context: Context,
    viewport: Rect,
    position: Position,
}
impl<'builder, 'render, 'handler> Builder<'builder, 'render, 'handler> {
    pub(crate) fn new(store: &'builder mut BuildStore<'render, 'handler>, viewport: Rect) -> Self {
        Builder {
            store,
            id_prefix: String::new(),
            context: Context::None,
            viewport,
            position: Position::new(viewport.x, viewport.y),
        }
    }

    pub fn context(&self) -> Context {
        self.context
    }

    pub fn theme(&self) -> &Theme {
        self.store.theme
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn position(&self) -> Position {
        self.position
    }

    pub fn set_position(&mut self, position: Position) {
        self.position = position;
    }

    pub fn take_lines(&mut self, lines: u16) -> Rect {
        if self.position.x > self.viewport.x {
            self.position.x = self.viewport.x;
            self.position.y += 1;
        }

        let lines = std::cmp::min(lines, self.viewport.height - self.position.y.saturating_sub(self.viewport.y));

        let area = Rect {
            x: self.viewport.x,
            y: self.position.y,
            width: self.viewport.width,
            height: lines,
        };
        self.position.y += lines;

        area
    }

    pub fn has_focus(&self, id: WidgetId) -> bool {
        matches!(
            self.store.ids.current.focus,
            Some(Focus { ghost: None, index }) if self.store.ids.current.focus_chain[index] == id)
    }

    pub fn add_widget_impl(&mut self, mut id: Cow<'_, str>, can_focus: bool) -> (WidgetId, Option<WidgetId>)
    {
        assert!(!id.is_empty());
        assert!(id.find("##").is_none(), "id cannot contain '##'");

        if !self.id_prefix.is_empty() {
            id = format!("{}-##-{}", self.id_prefix, id).into();
        }

        let previous = &mut self.store.ids.previous;
        let current = &mut self.store.ids.current;

        let old_id = previous.id_map.get(&*id).map(|x| *x);
        let new_id = WidgetId(current.ids.len());

        let id = id.into_owned();
        let old = current.id_map.insert(id.clone(), new_id);
        assert!(old.is_none());

        if can_focus {
            let focus = match (&current.focus, &mut previous.focus) {
                // If nothing was ever previously focused, the first
                // focusable widget takes the focus.
                (None, None) => Some(None),

                // If this widget had the focus before turning into a ghost
                // focus, reclaim the focus.
                //
                // The ghost location could be at an earlier widget, so we
                // need to check the current focus as well.
                (None, Some(Focus { ghost: Some(ref ghost_id), .. })) |
                (Some(Focus { ghost: Some(ref ghost_id), .. }), _)
                    if id == *ghost_id => Some(None),

                // If this widget was previously focus or carries the ghost
                // location, carry the focus / ghost location forward.
                (None, Some(Focus { ghost, index }))
                    if Some(previous.focus_chain[*index]) == old_id => Some(ghost.take()),

                _ => None,
            };
            if let Some(ghost) = focus {
                current.focus = Some(Focus { ghost, index: current.focus_chain.len() });
            }

            current.focus_chain.push(new_id);
        }

        current.ids.push(id);

        (new_id, old_id)
    }

    pub fn add_widget<'add, Id>(&'add mut self, id: Id, can_focus: bool) -> WidgetId
    where
        Id: Into<Cow<'add, str>>,
    {
        self.add_widget_impl(id.into(), can_focus).0
    }

    pub fn add_state_widget<'add, S, Id>(
        &'add mut self,
        id: Id,
        can_focus: bool,
    ) -> (WidgetId, &'handler mut S)
    where
        Id: Into<Cow<'add, str>>,
        S: Default + 'static,
    {
        let (new_id, old_id) = self.add_widget_impl(id.into(), can_focus);

        (
            new_id,
            self.store.state_builder.get_or_insert_default(new_id, old_id),
        )
    }

    pub fn add_state_widget_with<'add, S, Id, F>(
        &'add mut self,
        id: Id,
        can_focus: bool,
        f: F,
    ) -> (WidgetId, &'handler mut S)
    where
        Id: Into<Cow<'add, str>>,
        S: Default + 'static,
        F: FnOnce() -> S,
    {
        let (new_id, old_id) = self.add_widget_impl(id.into(), can_focus);

        (
            new_id,
            self.store.state_builder.get_or_insert_with(new_id, old_id, f),
        )
    }

    pub fn add_render(&mut self, renderable: Renderable<'render>) -> RenderId {
        self.store.render.push(renderable);
        RenderId(self.store.render.len() - 1)
    }

    pub fn get_render_mut(&mut self, id: RenderId) -> Option<&mut Renderable<'render>> {
        self.store.render.get_mut(id.0)
    }

    pub fn add_event_handler<H: (FnMut(&Event) -> Handled) + 'handler>(&mut self, h: H) {
        self.store.handlers.push(Box::new(h));
    }

    pub fn add_mouse_capture_handler<H: (FnMut(&Event) -> Handled) + 'handler>(&mut self, h: H) {
        self.store.handlers.insert(0, Box::new(h));
    }

    pub fn nest<'nest>(&'nest mut self) -> Nest<'nest, 'render, 'handler> {
        Nest {
            initial_focus_chain_len: self.store.ids.current.focus_chain.len(),
            builder: Builder { 
                store: self.store,
                id_prefix: self.id_prefix.clone(),
                context: self.context,
                viewport: self.viewport,
                position: self.position,
            },
        }
    }
}

pub struct Nest<'nest, 'render, 'handler> {
    builder: Builder<'nest, 'render, 'handler>,
    initial_focus_chain_len: usize,
}
impl<'nest, 'render, 'handler> Nest<'nest, 'render, 'handler> {
    pub fn build<F>(mut self, f: F) -> NestResult
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>),
    {
        f(&mut self.builder);

        let has_focus =
            self.builder.store.ids.current.focus.as_ref()
                .filter(|focus| focus.ghost.is_none())
                .map(|focus| focus.index >= self.initial_focus_chain_len)
                .unwrap_or(false);

        NestResult {
            has_focus,
        }
    }

    pub fn id(self, id: WidgetId) -> Self {
        assert!(id.0 == self.builder.store.ids.current.ids.len() - 1);

        let id_prefix = self.builder.store.ids.current.ids[id.0].clone();

        Nest {
            builder: Builder {
                id_prefix,
                ..self.builder
            },
            ..self
        }
    }

    pub fn viewport(self, viewport: Rect) -> Self {
        Nest {
            builder: Builder {
                viewport,
                position: Position::new(viewport.x, viewport.y),
                ..self.builder
            },
            ..self
        }
    }

    pub fn context(self, context: Context) -> Self {
        Nest {
            builder: Builder {
                context,
                ..self.builder
            },
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NestResult {
    pub has_focus: bool,
}
