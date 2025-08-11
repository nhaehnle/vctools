// SPDX-License-Identifier: GPL-3.0-or-later

use std::{any::Any, borrow::Cow, collections::HashMap, ops::Range};

use ratatui::{
    layout::{Position, Rect}, style::Style, widgets::{Block, Clear}, Frame
};

use vctuik_unsafe_internals::state;

use crate::{
    event::{Event, EventExt, KeyCode, KeyEventKind, KeySequence, MouseButton, MouseEventKind},
    layout::{Constraint1D, LayoutCache, LayoutEngine, LayoutItem1D},
    theme::{Context, Theme}
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateId(usize);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Focus {
    /// Modal context for the focus.
    modal: String,

    /// Index into the focus chain.
    index: usize,

    /// Whether the focus was dropped.
    dropped: bool,
}

#[derive(Debug)]
struct IdEntry {
    name: String,
    other_id: Option<StateId>,
}

#[derive(Default)]
struct IdState {
    id_map: HashMap<String, StateId>,
    ids: Vec<IdEntry>,
    modal: String,
    focus_chain: Vec<StateId>,
    focus: Vec<Focus>,
}
impl IdState {
    fn clear(&mut self) {
        self.id_map.clear();
        self.ids.clear();
        self.modal.clear();
        self.focus_chain.clear();
        self.focus.clear();
    }

    fn is_group_focus_index(&self, chain_idx: usize) -> bool {
        assert!(chain_idx < self.focus_chain.len());

        if chain_idx + 1 < self.focus_chain.len() {
            let id = self.focus_chain[chain_idx];
            let name = &self.ids[id.0].name;
            let next_id = self.focus_chain[chain_idx + 1];
            let next_name = &self.ids[next_id.0].name;
            if let Some(suffix) = next_name.strip_prefix(name) {
                return suffix.starts_with("-##-");
            }
        }

        return false;
    }

    fn focus_chain_index_has_prefix(&self, chain_idx: usize, prefix: &str) -> bool {
        let name = &self.ids[self.focus_chain[chain_idx].0].name;
        name.strip_prefix(prefix)
            .map(|suffix| suffix.starts_with("-##-"))
            .unwrap_or(false)
    }

    fn focus_chain_first(&self, prefix: &str) -> Option<usize> {
        if prefix.is_empty() {
            (self.focus_chain.len() > 0)
                .then_some(0)
        } else {
            (0..self.focus_chain.len()).position(|index| {
                self.focus_chain_index_has_prefix(index, prefix)
            })
        }
    }

    fn focus_chain_range(&self, prefix: &str) -> Range<usize> {
        if prefix.is_empty() {
            0..self.focus_chain.len()
        } else {
            let mut iter = (0..self.focus_chain.len()).peekable();
            while iter.peek().is_some_and(|&i| !self.focus_chain_index_has_prefix(i, prefix)) {
                iter.next();
            }
            let start = *iter.peek().unwrap_or(&self.focus_chain.len());

            while iter.peek().is_some_and(|&i| self.focus_chain_index_has_prefix(i, prefix)) {
                iter.next();
            }
            let end = *iter.peek().unwrap_or(&self.focus_chain.len());

            start..end
        }
    }
}

#[derive(Default)]
struct IdStore {
    previous: IdState,
    current: IdState,
}
impl IdStore {
    pub fn add_state_id(&mut self, name: String) -> StateId {
        let previous = &mut self.previous;
        let current = &mut self.current;

        let old_id = previous.id_map.get(&*name).map(|x| *x);
        let new_id = StateId(current.ids.len());

        if let Some(old_id) = old_id {
            previous.ids[old_id.0].other_id = Some(new_id);
        }

        let name: String = name.into();
        assert!(!self.current.id_map.contains_key(&name));
        self.current.id_map.insert(name.clone(), new_id);
        self.current.ids.push(IdEntry { name, other_id: old_id });

        new_id
    }
}

#[derive(Default)]
struct LayoutStore {
    previous: LayoutCache<StateId>,
    current: LayoutCache<StateId>,
}

#[derive(Default)]
pub(crate) struct Store {
    ids: IdStore,
    layout: LayoutStore,
    state: state::Store<StateId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusAction {
    None,
    Grab(StateId),
    Drop(StateId),
}

pub(crate) struct BuildStore<'store, 'frame> {
    ids: &'store mut IdStore,
    layout: &'store mut LayoutStore,
    state_builder: state::Builder<'store, StateId>,
    pub(crate) frame: &'store mut Frame<'frame>,
    theme: &'store Theme,
    event: Option<EventExt>,
    event_handled: bool,
    pub(crate) injected: Vec<Box<dyn Any + Send + Sync>>,
    pub(crate) need_refresh: bool,
    focus_action: FocusAction,
}
impl<'store, 'frame> BuildStore<'store, 'frame> {
    pub(crate) fn new(state: &'store mut Store, theme: &'store Theme,
                      frame: &'store mut Frame<'frame>,
                      event: Option<EventExt>) -> Self {
        let ids = &mut state.ids;
        let layout = &mut state.layout;
        let state_builder = state::Builder::new(&mut state.state);

        BuildStore {
            ids,
            layout,
            state_builder,
            frame,
            theme,
            event,
            event_handled: false,
            injected: Vec::new(),
            need_refresh: false,
            focus_action: FocusAction::None,
        }
    }

    fn event_handled(&mut self) -> bool {
        let ret = !self.event_handled;
        self.event_handled = true;
        ret
    }

    pub fn current_layout_mut(&mut self) -> &mut LayoutCache<StateId> {
        &mut self.layout.current
    }

    fn find_old_id_in_focus_chain(&self, old_id: StateId, cur_range: Range<usize>) -> Option<usize> {
        self.ids.previous.ids[old_id.0].other_id
            .and_then(|new_id| {
                self.ids.current.focus_chain[cur_range.clone()]
                    .iter()
                    .enumerate()
                    .find(|(_, id)| **id == new_id)
                    .map(|(index, _)| cur_range.start + index)
            })
    }

    pub fn end_frame(&mut self) {
        // Preserve the previous focus hierarchy.
        let mut new_focus = Vec::new();

        {
            let mut cut_off = false;
            let mut last_preserved = true;

            for focus in std::mem::take(&mut self.ids.previous.focus) {
                let is_modal_prefix =
                    focus.modal.is_empty() ||
                    self.ids.current.modal.strip_prefix(&focus.modal)
                        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with("-##-"));
                if !is_modal_prefix {
                    // Modal changed in a way to make this and all children irrelevant.
                    cut_off = true;
                    break;
                }

                let prev_range = self.ids.previous.focus_chain_range(&focus.modal);
                let cur_range = self.ids.current.focus_chain_range(&focus.modal);

                if cur_range.is_empty() {
                    cut_off = true;
                    break;
                }

                assert!(prev_range.contains(&focus.index));

                let index =
                    // Preserve the exact focus if possible
                    self.find_old_id_in_focus_chain(self.ids.previous.focus_chain[focus.index], cur_range.clone())
                        .or_else(|| {
                            // Otherwise, first find the nearest ancestor that still exists.
                            let name = &self.ids.previous.ids[self.ids.previous.focus_chain[focus.index].0].name;
                            let ancestor: Option<(usize, &str)> =
                                self.ids.previous.focus_chain[prev_range.start..focus.index]
                                    .iter()
                                    .rev()
                                    .scan(name.as_str(), |prefix, old_id| {
                                        let earlier_name = &self.ids.previous.ids[old_id.0].name;
                                        let common_prefix_len =
                                            earlier_name.char_indices().zip(prefix.char_indices())
                                                .find_map(|((lhs_offset, lhs_ch), (rhs_offset, rhs_ch))| {
                                                    assert!(lhs_offset == rhs_offset);
                                                    if lhs_ch != rhs_ch {
                                                        Some(lhs_offset)
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .unwrap_or(std::cmp::min(earlier_name.len(), prefix.len()));
                                        *prefix = &prefix[..common_prefix_len];
                                        if prefix.is_empty() {
                                            None
                                        } else if prefix.len() == earlier_name.len() &&
                                                  name[prefix.len()..].starts_with("-##-") {
                                            Some(Some((*prefix, old_id)))
                                        } else {
                                            Some(None)
                                        }
                                    })
                                    .find_map(|maybe_ancestor| {
                                        if let Some((prefix, &old_id)) = maybe_ancestor {
                                            self.find_old_id_in_focus_chain(old_id, cur_range.clone())
                                                .map(|index| (index, prefix))
                                        } else {
                                            None
                                        }
                                    });

                            // Next, find the first subsequent ID that has a current
                            // correspondent under the ancestor.
                            let successor_new_idx =
                                self.ids.previous.focus_chain[focus.index + 1..prev_range.end]
                                    .iter()
                                    .take_while(|&old_id| {
                                        if let Some((_, prefix)) = ancestor {
                                            let name = &self.ids.previous.ids[old_id.0].name;
                                            return name.strip_prefix(prefix).is_some_and(|suffix| suffix.starts_with("-##-"));
                                        }
                                        true
                                    })
                                    .find_map(|&old_id| {
                                        self.find_old_id_in_focus_chain(old_id, cur_range.clone())
                                    });

                            // Prefer a successor under a shared parent; but if we didn't find one,
                            // just return the closest predecessor.
                            successor_new_idx
                                .or_else(|| {
                                    self.ids.previous.focus_chain[prev_range.start..focus.index]
                                        .iter()
                                        .rev()
                                        .find_map(|&old_id| {
                                            self.find_old_id_in_focus_chain(old_id, cur_range.clone())
                                        })
                                })
                        })
                        .unwrap_or(cur_range.start);

                assert!(cur_range.contains(&index));

                let old_id = self.ids.previous.focus_chain[focus.index];
                let new_id = self.ids.current.focus_chain[index];
                last_preserved = self.ids.current.ids[new_id.0].other_id == Some(old_id);

                new_focus.push(Focus {
                    modal: focus.modal,
                    index,
                    dropped: focus.dropped && last_preserved,
                });
            }

            if cut_off || !last_preserved {
                self.need_refresh = true;
            }
        }

        // If there is no focus, assign it to the first available item.
        if new_focus.last().is_none_or(|focus| focus.modal != self.ids.current.modal) {
            if let Some(first_idx) = self.ids.current.focus_chain_first(&self.ids.current.modal) {
                new_focus.push(Focus {
                    modal: self.ids.current.modal.clone(),
                    index: first_idx,
                    dropped: false,
                });
                self.need_refresh = true;
            }
        }

        // Handle focus actions.
        match self.focus_action {
            FocusAction::None => {},
            FocusAction::Grab(id) => {
                if let Some(chain_idx) = self.ids.current.focus_chain.iter().position(|x| *x == id) {
                    let name = &self.ids.current.ids[id.0].name;
                    if let Some(focus) = new_focus.iter_mut().rev().find(|focus| {
                        focus.modal.is_empty() ||
                        name.strip_prefix(&focus.modal).is_some_and(|suffix| suffix.starts_with("-##-"))
                    }) {
                        focus.index = chain_idx;
                        focus.dropped = false;
                    } else {
                        new_focus.insert(0, Focus {
                            modal: String::new(),
                            index: chain_idx,
                            dropped: false,
                        });
                    }
                    self.need_refresh = true;
                }
            },
            FocusAction::Drop(id) => {
                if let Some(focus) =
                        new_focus.iter_mut()
                            .find(|focus| self.ids.current.focus_chain[focus.index] == id) {
                    focus.dropped = true;
                    self.need_refresh = true;
                }
            }
        }

        // Ensure that we don't individually focus the parent of a group.
        if let Some(focus) = new_focus.last_mut() {
            while self.ids.current.is_group_focus_index(focus.index) {
                focus.index += 1;
                focus.dropped = false;
                self.need_refresh = true;
            }
        }

        // We handle keyboard events last, after all other widgets have had a chance to intercept
        // so that text fields can capture tabs.
        if !self.event_handled {
            let next = match self.event {
                Some(EventExt::Event(Event::Key(ev))) if ev.kind == KeyEventKind::Press => {
                    if ev.code == KeyCode::Tab ||
                       (ev.code == KeyCode::Down && ev.modifiers.is_empty()) {
                        Some(true)
                    } else if ev.code == KeyCode::BackTab ||
                              (ev.code == KeyCode::Up && ev.modifiers.is_empty()) {
                        Some(false)
                    } else {
                        None
                    }
                },
                _ => None,
            };

            if let Some((focus, next)) = new_focus.last_mut().zip(next) {
                let range = self.ids.current.focus_chain_range(&focus.modal);

                if next {
                    if !focus.dropped {
                        loop {
                            focus.index += 1;
                            if focus.index >= range.end {
                                focus.index = range.start;
                            }
                            if !self.ids.current.is_group_focus_index(focus.index) {
                                break;
                            }
                        }
                    }
                } else {
                    loop {
                        if focus.index == range.start {
                            focus.index = range.end - 1;
                            break;
                        }
                        focus.index -= 1;
                        if !self.ids.current.is_group_focus_index(focus.index) {
                            break;
                        }
                    }
                }

                focus.dropped = false;

                self.need_refresh = true;
            }
        }

        self.ids.current.focus = new_focus;

        // State double-buffer management
        self.layout.current.save_persistent(
            std::mem::take(&mut self.layout.previous),
            |old_id| {
                self.ids.previous.ids[old_id.0].other_id.unwrap_or_else(|| {
                    self.ids.add_state_id(self.ids.previous.ids[old_id.0].name.clone())
                })
            });

        self.ids.previous.clear();
        self.layout.previous.clear();
        for IdEntry { other_id, .. } in self.ids.current.ids.iter_mut() {
            *other_id = None;
        }
        std::mem::swap(&mut self.ids.previous, &mut self.ids.current);
        std::mem::swap(&mut self.layout.previous, &mut self.layout.current);
    }
}

pub struct Builder<'builder, 'store, 'frame> {
    store: &'builder mut BuildStore<'store, 'frame>,
    name_prefix: String,
    theme_context: Context,
    viewport: Rect,
    layout: &'builder mut LayoutEngine<StateId>,
}
impl<'builder, 'store, 'frame> Builder<'builder, 'store, 'frame> {
    pub(crate) fn new(store: &'builder mut BuildStore<'store, 'frame>, layout: &'builder mut LayoutEngine<StateId>, viewport: Rect) -> Self {
        Builder {
            store,
            name_prefix: String::new(),
            theme_context: Context::None,
            viewport,
            layout,
        }
    }

    pub fn frame<'slf>(&'slf mut self) -> &'slf mut Frame<'frame> {
        &mut self.store.frame
    }

    pub fn theme_context(&self) -> Context {
        self.theme_context
    }

    pub fn theme(&self) -> &Theme {
        self.store.theme
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn is_at_top(&self) -> bool {
        self.layout.position() == 0
    }

    pub fn take_lines(&mut self, item: LayoutItem1D<StateId>) -> Rect {
        let old_id = item.id.and_then(|id| self.store.ids.current.ids[id.0].other_id);
        let (pos, size) = self.layout.add(&self.store.layout.previous, old_id, item);

        let rel_y = std::cmp::min(pos, self.viewport.height);
        let height = std::cmp::min(size, self.viewport.height - rel_y);

        let area = Rect {
            x: self.viewport.x,
            y: self.viewport.y + rel_y,
            width: self.viewport.width,
            height,
        };

        area
    }

    pub fn take_lines_fixed(&mut self, lines: u16) -> Rect {
        self.take_lines(LayoutItem1D::new(Constraint1D::new_fixed(lines)))
    }

    pub fn add_slack(&mut self) {
        let state_id = self.add_state_id("_slack");
        self.take_lines(LayoutItem1D::new(Constraint1D::unconstrained()).id(state_id, true));
    }

    pub fn layout_drag(&mut self, y: u16, delta: i16) {
        self.layout.drag(y, delta);
    }

    /// Register the given ID as being able to receive group focus.
    ///
    /// This means that the ID can be focused individually, but only if there
    /// are no focusable child IDs.
    ///
    /// Returns true if any child has focus.
    pub fn check_group_focus(&mut self, id: StateId) -> bool {
        assert!(self.store.ids.current.ids[id.0].name == self.name_prefix);

        let self_focus = self.check_focus(id);
        self_focus || self.has_group_focus()
    }

    pub fn has_group_focus(&self) -> bool {
        let name =
            self.store.ids.previous.focus
                .last()
                .map(|focus| {
                    let id = self.store.ids.previous.focus_chain[focus.index];
                    &self.store.ids.previous.ids[id.0].name
                });
        name.and_then(|name| {
            name.strip_prefix(&self.name_prefix)
                .map(|suffix| suffix.starts_with("-##-"))
        }).unwrap_or(false)
    }

    /// Register the given ID as being able to receive focus and check whether it
    /// has focus.
    pub fn check_focus(&mut self, id: StateId) -> bool {
        // Register the state ID as being able to receive focus.
        self.store.ids.current.focus_chain.push(id);

        // Check whether we should be in focus, based on the last frame.
        //
        // At this point, we specifically do *not*
        //  - automatically re-focus ghosts or
        //  - automatically focus the first item in the chain.
        // Doing so might cause inconsistencies with earlier has_group_focus checks.
        if !self.store.ids.current.ids[id.0].name.starts_with(&self.store.ids.previous.modal) {
            return false;
        }

        let Some(old_focus) = self.store.ids.previous.focus.last() else {
            return false;
        };

        if old_focus.dropped {
            return false;
        }

        if let Some(old_id) = self.store.ids.current.ids[id.0].other_id {
            return self.store.ids.previous.focus_chain[old_focus.index] == old_id;
        }

        false
    }

    fn set_focus_action(&mut self, action: FocusAction) {
        assert!(self.store.focus_action == FocusAction::None);
        self.store.focus_action = action;
    }

    pub fn grab_focus(&mut self, id: StateId) {
        self.set_focus_action(FocusAction::Grab(id));
    }

    pub fn drop_focus(&mut self, id: StateId) {
        self.set_focus_action(FocusAction::Drop(id));
    }

    pub fn need_refresh(&mut self) {
        self.store.need_refresh = true;
    }

    pub fn inject_custom<T: Sync + Send + 'static>(&mut self, event: T) {
        self.store.injected.push(Box::new(event));
    }

    pub fn peek_event(&self) -> Option<&Event> {
        self.store.event.as_ref().and_then(|ext| match ext {
            EventExt::Event(event) => Some(event),
            _ => None,
        })
    }

    pub fn with_event<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&Event) -> Option<R>,
    {
        if !self.store.event_handled {
            if let Some(EventExt::Event(event)) = &self.store.event {
                if let Some(result) = f(event) {
                    self.store.event_handled = true;
                    return Some(result);
                }
            }
        }
        None
    }

    pub fn on_custom<T: 'static>(&mut self) -> Option<&T> {
        if !self.store.event_handled {
            if let Some(EventExt::Custom(data)) = &self.store.event {
                if let Some(result) = data.downcast_ref::<T>() {
                    self.store.event_handled = true;
                    return Some(result);
                }
            }
        }
        None
    }

    pub fn on_key_press(&mut self, key_seq: impl Into<KeySequence>) -> bool {
        let key_seq = key_seq.into();
        matches!(
            self.store.event,
            Some(EventExt::Event(Event::Key(ev))) if ev.kind == KeyEventKind::Press && key_seq.matches(&ev)) &&
            self.store.event_handled()
    }

    pub fn on_key_press_any(&mut self, key_seqs: &[KeySequence]) -> bool {
        match self.store.event {
        Some(EventExt::Event(Event::Key(ev))) if ev.kind == KeyEventKind::Press => {
            key_seqs.iter().any(|seq| seq.matches(&ev)) && self.store.event_handled()
        },
        _ => false,
        }
    }

    pub fn on_mouse_press(&mut self, area: Rect, button: MouseButton) -> Option<Position> {
        match self.store.event {
            Some(EventExt::Event(Event::Mouse(ev))) if ev.kind == MouseEventKind::Down(button) => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_down(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(EventExt::Event(Event::Mouse(ev))) if ev.kind == MouseEventKind::ScrollDown => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_up(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(EventExt::Event(Event::Mouse(ev))) if ev.kind == MouseEventKind::ScrollUp => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_left(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(EventExt::Event(Event::Mouse(ev))) if ev.kind == MouseEventKind::ScrollLeft => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn on_mouse_scroll_right(&mut self, area: Rect) -> Option<Position> {
        match self.store.event {
            Some(EventExt::Event(Event::Mouse(ev))) if ev.kind == MouseEventKind::ScrollRight => {
                let pos = Position::new(ev.column, ev.row);
                area.contains(pos).then_some(pos).filter(|_| self.store.event_handled())
            },
            _ => None,
        }
    }

    pub fn add_state_id_impl(&mut self, mut name: Cow<'_, str>) -> StateId {
        assert!(!name.is_empty());
        assert!(name.find("##").is_none(), "id cannot contain '##'");

        if !self.name_prefix.is_empty() {
            name = format!("{}-##-{}", self.name_prefix, name).into();
        }

        self.store.ids.add_state_id(name.into())
    }

    pub fn add_state_id<'name>(&mut self, name: impl Into<Cow<'name, str>>) -> StateId {
        self.add_state_id_impl(name.into())
    }

    pub fn get_state<'add, S>(&mut self, id: StateId) -> &'add mut S
    where
        S: Default + 'static,
        'store: 'add,
    {
        let old_id = self.store.ids.current.ids[id.0].other_id;
        self.store.state_builder.get_or_insert_default(id, old_id)
    }

    pub fn nest<'nest>(&'nest mut self) -> Nest<'nest, 'store, 'frame> {
        Nest {
            parent: None,
            builder: Builder { 
                store: self.store,
                name_prefix: self.name_prefix.clone(),
                theme_context: self.theme_context,
                viewport: self.viewport,
                layout: self.layout,
            },
            modal: false,
            popup: None,
        }
    }
}

pub struct Nest<'nest, 'store, 'frame> {
    builder: Builder<'nest, 'store, 'frame>,
    parent: Option<StateId>,
    modal: bool,
    popup: Option<(Style, u16, &'nest mut u16)>,
}
impl<'nest, 'store, 'frame> Nest<'nest, 'store, 'frame> {
    pub fn build<F, R>(self, f: F) -> R
    where
        F: FnOnce(&mut Builder) -> R,
    {
        if self.modal {
            assert!(
                self.builder.store.ids.current.modal.is_empty() ||
                self.builder.name_prefix.strip_prefix(&self.builder.store.ids.current.modal)
                    .is_some_and(|suffix| suffix.starts_with("-##-")));
            self.builder.store.ids.current.modal = self.builder.name_prefix.to_string();
        }

        if let Some((style, max_height, out_height)) = self.popup {
            let mut layout = LayoutEngine::new();
            let mut builder = Builder {
                layout: &mut layout,
                ..self.builder
            };

            let area = builder.viewport();
            builder.frame().render_widget(Clear, area);
            builder.frame().render_widget(Block::new().style(style), area);

            let result = f(&mut builder);

            let (changed, height) =
                std::mem::take(builder.layout).finish(Constraint1D::new(0, max_height), &mut builder.store.layout.current);
            *out_height = height;
            if changed || height != area.height {
                builder.store.need_refresh = true;
            }

            result
        } else {
            f(&mut Builder { ..self.builder })
        }
    }

    pub fn id(self, id: StateId) -> Self {
        assert!(id.0 == self.builder.store.ids.current.ids.len() - 1);
        assert!(self.parent.is_none());

        let name_prefix = self.builder.store.ids.current.ids[id.0].name.clone();

        Nest {
            builder: Builder {
                name_prefix,
                ..self.builder
            },
            parent: Some(id),
            ..self
        }
    }

    pub fn modal(self, id: StateId, modal: bool) -> Self {
        Nest {
            modal,
            ..self.id(id)
        }
    }

    pub fn popup(self, area: Rect, style: Style, max_height: u16, out_height: &'nest mut u16) -> Self {
        Nest {
            builder: Builder {
                viewport: area,
                ..self.builder
            },
            popup: Some((style, max_height, out_height)),
            ..self
        }
    }

    pub fn theme_context(self, theme_context: Context) -> Self {
        Nest {
            builder: Builder {
                theme_context,
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
