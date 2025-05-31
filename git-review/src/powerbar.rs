// SPDX-License-Identifier: GPL-3.0-or-later

use ratatui::prelude::*;
use vctuik::{prelude::*, state::Builder};

pub trait PowerBarSource {
    fn num_popup_lines(&self, viewport: Rect) -> u16;
    fn build_popup(&mut self, builder: &mut Builder<'_, '_, '_>);
}

pub struct PowerBarState {
    source: Option<Box<dyn PowerBarSource>>,
}
impl PowerBarState {
    pub fn new() -> Self {
        PowerBarState { source: None }
    }

    pub fn take(&mut self, source: impl PowerBarSource) {
        self.source = Some(Box::new(source));
    }
}

pub struct PowerBar {}
impl PowerBar {
    pub fn new() -> Self {
        PowerBar {}
    }
}
