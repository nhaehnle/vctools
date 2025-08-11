// SPDX-License-Identifier: GPL-3.0-or-later

use std::{borrow::Cow, hash::Hash};

pub use tui_tree_widget::{Tree, TreeItem};
use tui_tree_widget::TreeState;

use crate::{
    event::{KeyCode, MouseButton},
    layout::{Constraint1D, LayoutItem1D}, state::Builder
};

pub trait TreeBuild<TreeId> {
    fn build<'id, Id>(
        self,
        builder: &mut Builder,
        id: Id)
    where
        Id: Into<Cow<'id, str>>;
}
impl<'tree, TreeId> TreeBuild<TreeId> for Tree<'tree, TreeId>
where
    TreeId: Default + Clone + PartialEq + Eq + Hash + 'static,
{
    fn build<'id, Id>(
        self,
        builder: &mut Builder,
        id: Id)
    where
        Id: Into<Cow<'id, str>>,
    {
        let state_id = builder.add_state_id(id.into());
        let state: &mut TreeState<TreeId> = builder.get_state(state_id);
        let area = builder.take_lines(LayoutItem1D::new(Constraint1D::new_min(5)).id(state_id, true));

        // Handle events
        let has_focus = builder.check_focus(state_id);

        if has_focus {
            if builder.on_key_press(KeyCode::Left) { state.key_left(); }
            if builder.on_key_press(KeyCode::Right) { state.key_right(); }
            if builder.on_key_press(KeyCode::Down) { state.key_down(); }
            if builder.on_key_press(KeyCode::Up) { state.key_up(); }
            if builder.on_key_press(KeyCode::Esc) { state.select(Vec::new()); }
            if builder.on_key_press(KeyCode::Home) { state.select_first(); }
            if builder.on_key_press(KeyCode::End) { state.select_last(); }
            if builder.on_key_press(KeyCode::PageDown) { for _ in 0..(area.height.div_ceil(3)) { state.key_down(); } }
            if builder.on_key_press(KeyCode::PageUp) { for _ in 0..(area.height.div_ceil(3)) { state.key_up(); } }
        }

        if let Some(pos) = builder.on_mouse_press(area, MouseButton::Left) {
            state.click_at(pos);
        }
        if builder.on_mouse_scroll_up(area).is_some() {
            for _ in 0..(area.height.div_ceil(5)) { state.key_up(); }
        }
        if builder.on_mouse_scroll_down(area).is_some() {
            for _ in 0..(area.height.div_ceil(5)) { state.key_down(); }
        }

        // Render tree
        let text_theme = builder.theme().text(builder.theme_context());
        let mut selected = text_theme.selected;
        if has_focus {
            selected = selected.patch(text_theme.highlight);
        }
        let tree = self
            .style(text_theme.normal)
            .highlight_style(selected);

        builder.frame().render_stateful_widget(tree, area, state);
    }
}
