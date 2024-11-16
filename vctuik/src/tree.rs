use std::{borrow::Cow, cell::RefCell, hash::Hash};

use ratatui::layout::Position;
pub use tui_tree_widget::{Tree, TreeItem};
use tui_tree_widget::TreeState;

use crate::{
    event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind},
    state::{Builder, Handled, Renderable},
};

pub trait TreeBuild<'render, TreeId> {
    fn build<'a, 'handler, 'id, Id>(
        self,
        builder: &'a mut Builder<'_, 'render, 'handler>,
        id: Id,
        num_lines: u16)
    where
        Id: Into<Cow<'id, str>>,
        'handler: 'render;
}
impl<'render, TreeId> TreeBuild<'render, TreeId> for Tree<'render, TreeId>
where
    TreeId: Default + Clone + PartialEq + Eq + Hash + 'static,
{
    fn build<'a, 'handler, 'id, Id>(
        self,
        builder: &'a mut Builder<'_, 'render, 'handler>,
        id: Id,
        num_lines: u16)
    where
        Id: Into<Cow<'id, str>>,
        'handler: 'render,
    {
        let (id, state): (_, &mut RefCell<TreeState<TreeId>>) = builder.add_state_widget(id.into(), true);
        let has_focus = builder.has_focus(id);
        let state: &_ = state;

        let text_theme = builder.theme().text(builder.context());
        let mut selected = text_theme.selected;
        if has_focus {
            selected = selected.patch(text_theme.highlight);
        }
        let tree = self
            .style(text_theme.normal)
            .highlight_style(selected);
    
        let area = builder.take_lines(num_lines);
        builder.add_render(Renderable::Other(Box::new({
            move |frame| {
                frame.render_stateful_widget(tree, area, &mut state.borrow_mut());
            }
        })));

        builder.add_event_handler(move |ev| {
            let mut state = state.borrow_mut();
            match ev {
                Event::Key(key) if has_focus && key.kind == KeyEventKind::Press => {
                    match key.code {
                        KeyCode::Left => { state.key_left(); },
                        KeyCode::Right => { state.key_right(); },
                        KeyCode::Down => { state.key_down(); },
                        KeyCode::Up => { state.key_up(); },
                        KeyCode::Esc => { state.select(Vec::new()); },
                        KeyCode::Home => { state.select_first(); },
                        KeyCode::End => { state.select_last(); },
                        KeyCode::PageDown => { for _ in 0..(area.height.div_ceil(3)) { state.key_down(); } },
                        KeyCode::PageUp => { for _ in 0..(area.height.div_ceil(3)) { state.key_up(); } },
                        _ => return Handled::No,
                    }
                    Handled::Yes
                }
                Event::Mouse(ev) if area.contains(Position::new(ev.column, ev.row)) => {
                    match ev.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            state.click_at(Position::new(ev.column, ev.row));
                            Handled::Yes
                        }
                        MouseEventKind::ScrollUp => {
                            for _ in 0..(area.height.div_ceil(5)) { state.key_up(); }
                            Handled::Yes
                        }
                        MouseEventKind::ScrollDown => {
                            for _ in 0..(area.height.div_ceil(5)) { state.key_down(); }
                            Handled::Yes
                        }
                        _ => Handled::No
                    }
                }
                _ => Handled::No,
            }
        });
    }
}
