// SPDX-License-Identifier: GPL-3.0-or-later

use ratatui::widgets::Block;
use vctuik::{
    event::KeyCode,
    layout::{Constraint1D, LayoutItem1D},
    state::Builder,
};

use tui_logger::{TuiLoggerWidget, TuiWidgetEvent, TuiWidgetState};

pub fn add_log_view(builder: &mut Builder) {
    let state_id = builder.add_state_id("logview");
    let state: &mut TuiWidgetState = builder.get_state(state_id);
    let has_focus = builder.check_focus(state_id);

    if has_focus {
        let event = if builder.on_key_press(KeyCode::Char(' ')) {
            Some(TuiWidgetEvent::SpaceKey)
        } else if builder.on_key_press(KeyCode::Down) {
            Some(TuiWidgetEvent::DownKey)
        } else if builder.on_key_press(KeyCode::Up) {
            Some(TuiWidgetEvent::UpKey)
        } else if builder.on_key_press(KeyCode::Left) {
            Some(TuiWidgetEvent::LeftKey)
        } else if builder.on_key_press(KeyCode::Right) {
            Some(TuiWidgetEvent::RightKey)
        } else if builder.on_key_press(KeyCode::Char('+')) {
            Some(TuiWidgetEvent::PlusKey)
        } else if builder.on_key_press(KeyCode::Char('-')) {
            Some(TuiWidgetEvent::MinusKey)
        } else if builder.on_key_press(KeyCode::Char('h')) {
            Some(TuiWidgetEvent::HideKey)
        } else if builder.on_key_press(KeyCode::Char('f')) {
            Some(TuiWidgetEvent::FocusKey)
        } else if builder.on_key_press(KeyCode::PageDown) {
            Some(TuiWidgetEvent::NextPageKey)
        } else if builder.on_key_press(KeyCode::PageUp) {
            Some(TuiWidgetEvent::PrevPageKey)
        } else {
            None
        };

        if let Some(event) = event {
            state.transition(event);
        }
    }

    let area = builder.take_lines(LayoutItem1D::new(Constraint1D::new_min(5)).id(state_id, true));

    let block = Block::default().style(builder.theme().pane_background);
    builder.frame().render_widget(block, area);

    let text = builder.theme().text(builder.theme_context());
    let logger = TuiLoggerWidget::default()
        .style(text.normal)
        .style_warn(text.highlight)
        .style_error(text.error)
        .state(state);
    builder.frame().render_widget(logger, area);
}
