use std::borrow::Cow;

use crate::{state::{Builder, Renderable}, theme::Context};

use ratatui::{layout::Rect, widgets::{Block, BorderType, Borders}};

#[derive(Debug, Clone)]
pub struct Constraint {
    pub min: Option<u16>,
    pub max: Option<u16>,
    pub fill: Option<u16>,
}

pub struct Pane {
    title: String,
}
impl Pane {
    pub fn new<T>(title: T) -> Self
    where 
        T: Into<String>
    {
        Self { title: title.into() }
    }
}

struct CompletePane<'panes, 'render, 'handler> {
    pane: Pane,
    build: Box<dyn FnOnce(&mut Builder<'_, 'render, 'handler>) + 'panes>,
}

pub struct Panes<'panes, 'render, 'handler> {
    panes: Vec<CompletePane<'panes, 'render, 'handler>>,
}
impl<'panes, 'render, 'handler> Panes<'panes,'render, 'handler> {
    pub fn new() -> Self {
        Self { panes: Vec::new() }
    }

    pub fn add<F>(&mut self, pane: Pane, build: F)
    where
        F: FnOnce(&mut Builder<'_, 'render, 'handler>) + 'panes,
    {
        self.panes.push(CompletePane { pane, build: Box::new(build) });
    }

    pub fn build<'id, Id>(self, builder: &mut Builder<'_, 'render, 'handler>, id: Id, num_lines: u16)
    where
        Id: Into<Cow<'id, str>>,
    {
        if self.panes.is_empty() {
            return;
        }

        let id = id.into();
        let clamped_panes = std::cmp::min(self.panes.len(), u16::MAX as usize) as u16;
        let pane_height = num_lines / clamped_panes;
        let remaining_height = num_lines - (pane_height * clamped_panes);

        assert!(remaining_height < clamped_panes);

        let layout: Vec<u16> =
            (0..remaining_height).map(|_| pane_height + 1)
                .chain(((remaining_height as usize)..self.panes.len()).map(|_| pane_height))
                .collect();

        let id = builder.add_id(id, false);

        builder.nest().id(id).build(|builder| {
            for (pane, height) in self.panes.into_iter().zip(layout.into_iter()) {
                let area = builder.take_lines(height);
                if area.is_empty() {
                    break
                }

                let inner_area = Rect::new(area.x, area.y + 1, area.width, area.height - 1);

                let render = builder.add_render(Renderable::None);

                let id = builder.add_id(&pane.pane.title, false);
                let nest = builder.nest().id(id).context(Context::Pane).viewport(inner_area).build(pane.build);

                let mut block = Block::default()
                    .title(pane.pane.title)
                    .borders(Borders::TOP)
                    .style(builder.theme().pane_background);

                if nest.has_focus {
                    block = block.border_type(BorderType::Thick);
                    block = block.border_style(builder.theme().pane_frame_focus);
                } else {
                    block = block.border_type(BorderType::Plain);
                    block = block.border_style(builder.theme().pane_frame_normal);
                }

                *builder.get_render_mut(render).unwrap() = Renderable::Block(area, block);
            }
        });
    }
}
