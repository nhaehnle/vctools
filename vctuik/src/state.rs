use std::{any::Any, borrow::Cow, collections::HashMap};

use ratatui::{
    layout::{Position, Rect},
    Frame,
};

use crate::{
    event::Event,
    theme::{Context, Theme},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    Yes,
    No,
}

pub trait IRenderable {
    fn render(&self, frame: &mut Frame);
}

pub enum Renderable<'render> {
    Span(Rect, ratatui::text::Span<'render>),
    Line(Rect, ratatui::text::Line<'render>),
    Text(Rect, ratatui::text::Text<'render>),
    Block(Rect, ratatui::widgets::Block<'render>),
    SetCursor(Position),
    Other(Box<dyn IRenderable + 'render>),
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
            Renderable::Other(other) => other.render(frame),
            Renderable::None => {},
        }
    }
}

struct StateNode {
    id: String,
    state: Option<Box<dyn Any>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Focus {
    ghost: Option<String>,
    index: usize,
}

#[derive(Default)]
pub struct StateNodes {
    entries: Vec<StateNode>,
    id_map: HashMap<String, usize>,
    focus_chain: Vec<StateId>,
    focus: Option<Focus>,
}
impl StateNodes {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.id_map.clear();
        self.focus_chain.clear();
        self.focus = None;
    }

    pub fn get_state<T: 'static>(&self, id: StateId) -> Option<&T> {
        self.entries
            .get(id.0)
            .and_then(|state| state.state.as_ref())
            .and_then(|state| state.downcast_ref())
    }

    pub fn can_focus(&self) -> bool {
        !self.focus_chain.is_empty()
    }

    pub fn move_focus(&mut self, next: bool) {
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
pub(crate) struct StateStore {
    pub(crate) previous: StateNodes,
    pub(crate) current: StateNodes,
}

pub(crate) type EventHandlers<'handler> = Vec<Box<dyn (FnMut(&Event) -> Handled) + 'handler>>;

pub(crate) struct BuildStore<'render, 'handler> {
    pub(crate) state: StateStore,
    pub(crate) render: Vec<Renderable<'render>>,
    pub(crate) handlers: EventHandlers<'handler>,
    pub(crate) theme: &'render Theme,
}
impl<'render, 'handler> BuildStore<'render, 'handler> {
    pub(crate) fn new(mut state: StateStore, theme: &'render Theme) -> Self {
        std::mem::swap(&mut state.previous, &mut state.current);
        state.current.clear();

        BuildStore {
            state,
            render: Vec::new(),
            handlers: Vec::new(),
            theme,
        }
    }

    pub(crate) fn finish(&mut self) {
        if self.state.current.focus.is_none() && !self.state.current.focus_chain.is_empty() {
            if let Some(Focus { mut ghost, index }) = self.state.previous.focus.take() {
                if ghost.is_none() {
                    ghost = Some(std::mem::take(&mut self.state.previous.entries[self.state.previous.focus_chain[index].0].id));
                }

                let ghost_index =
                    self.state.previous.focus_chain[(index + 1)..]
                        .iter()
                        .copied()
                        .filter_map(|old_id| {
                            self.state.current.id_map.get(&self.state.previous.entries[old_id.0].id)
                                .map(|&id| StateId(id))
                        })
                        .find_map(|new_id| {
                            self.state.current.focus_chain.iter()
                                .enumerate()
                                .find_map(|(index, &id)| (id == new_id).then_some(index))
                        })
                        .unwrap_or(0);
                self.state.current.focus = Some(Focus { ghost, index: ghost_index });
            }
        }
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

    pub fn has_focus(&self, id: StateId) -> bool {
        matches!(
            self.store.state.current.focus,
            Some(Focus { ghost: None, index }) if self.store.state.current.focus_chain[index] == id)
    }

    pub fn add_id<'a, Id>(&'a mut self, id: Id, can_focus: bool) -> StateId
    where
        Id: Into<Cow<'a, str>>,
    {
        self.add_id_with_state(id, can_focus, || ()).0
    }

    pub fn add_id_with_state<'add, Id, T, F>(
        &'add mut self,
        id: Id,
        can_focus: bool,
        f: F,
    ) -> (StateId, &'add mut T)
    where
        Id: Into<Cow<'add, str>>,
        F: FnOnce() -> T,
        T: 'static,
    {
        let mut id: Cow<'_, str> = id.into();

        assert!(!id.is_empty());
        assert!(id.find("##").is_none(), "id cannot contain '##'");

        if !self.id_prefix.is_empty() {
            id = format!("{}-##-{}", self.id_prefix, id).into();
        }

        let previous = &mut self.store.state.previous;
        let current = &mut self.store.state.current;

        let mut old_index = previous.id_map.get(&*id).map(|x| *x);
        let state = old_index
            .and_then(|index| previous.entries[index].state.take())
            .filter(|state| state.is::<T>());
        if state.is_none() {
            // In the unusual case that the state type has changed, we treat this
            // as a new widget.
            old_index = None;
        }

        let index = current.entries.len();
        let id = id.into_owned();
        let old = current.id_map.insert(id.clone(), index);
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
                    if Some(previous.focus_chain[*index].0) == old_index => Some(ghost.take()),

                _ => None,
            };
            if let Some(ghost) = focus {
                current.focus = Some(Focus { ghost, index: current.focus_chain.len() });
            }

            current.focus_chain.push(StateId(index));
        }

        current.entries.push(StateNode {
            id,
            state,
        });

        (
            StateId(index),
            current.entries[index].state
                .get_or_insert_with(|| Box::new(f()))
                .downcast_mut().unwrap(),
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

    pub fn nest<'nest>(&'nest mut self) -> Nest<'nest, 'render, 'handler> {
        Nest {
            initial_focus_chain_len: self.store.state.current.focus_chain.len(),
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
            self.builder.store.state.current.focus.as_ref()
                .filter(|focus| focus.ghost.is_none())
                .map(|focus| focus.index >= self.initial_focus_chain_len)
                .unwrap_or(false);

        NestResult {
            has_focus,
        }
    }

    pub fn id(self, id: StateId) -> Self {
        assert!(id.0 == self.builder.store.state.current.entries.len() - 1);

        let id_prefix = self.builder.store.state.current.entries[id.0].id.clone();

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
