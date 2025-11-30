// SPDX-License-Identifier: GPL-3.0-or-later

use ratatui::widgets::Block;

use vctuik::{
    check_box::add_check_box,
    event::KeyCode,
    input::Input,
    label::add_label,
    layout::Constraint1D,
    prelude::*,
    section::with_section,
    table::{self, simple_table},
};

fn main() -> Result<()> {
    let mut terminal = vctuik::init()?;

    let mut running = true;

    let mut foo = false;
    let mut bar = false;
    let mut name: String = "world".into();
    let mut last_event = None;

    let mut table_source_state = simple_table::SourceState::new();

    terminal.run(|builder| {
        // Clear the window
        let frame_area = builder.frame().area();
        let block = Block::new().style(builder.theme().pane_background);
        builder.frame().render_widget(block, frame_area);

        // Draw UI
        with_section(builder, "Settings", |builder| {
            add_check_box(builder, "Foo", &mut foo);
            add_check_box(builder, "Bar", &mut bar);
            Input::new("name").label("Name:").build(builder, &mut name);
            builder.add_slack();
        });

        with_section(builder, "Commentary", |builder| {
            add_label(builder, "Cheddar");
            add_label(builder, "Provolone");
            add_label(builder, "Swiss");
            add_label(builder, format!("Hello, {name}!"));
            builder.add_slack();
        });

        with_section(builder, "Running", |builder| {
            add_check_box(builder, "Running", &mut running);
            if Input::new("name").build(builder, &mut name).is_some() {
                builder.need_refresh();
            }

            if let Some(ev) = builder.peek_event() {
                last_event = Some(ev.clone());
            }
            if let Some(ev) = last_event.as_ref() {
                add_label(builder, format!("Last event: {ev:?}"));
            } else {
                add_label(builder, "No events yet");
            }

            builder.add_slack();
        });

        with_section(builder, "Tree", |builder| {
            let mut source_builder = table_source_state.build();

            let id_style =
                source_builder.add_style(builder.theme().text(builder.theme_context()).header2);

            // The way in which items are added is intentionally interleaved in
            // a slightly weird way, to demonstrate the flexibility of the
            // `simple_table` API.
            let tl1 = source_builder.add(0, 0).raw(0, "Top-level 1").id();
            let tl2 = source_builder.add(0, 1).raw(0, "Top-level 2").id();
            let child1 = source_builder.add(tl1, 0).raw(0, "Child 1").id();
            source_builder.add(child1, 0).raw(0, "Grandchild 1");
            let child2 = source_builder.add(tl1, 1).raw(0, "Child 2").id();
            source_builder.add(child1, 1).raw(0, "Grandchild 2");
            source_builder.add(tl1, 2).raw(0, "Child 3");

            for idx in 0..25 {
                source_builder
                    .add(tl2, idx)
                    .raw(0, format!("Child {idx}"))
                    .styled(1, format!("{idx}"), id_style);
                source_builder
                    .add(child2, idx)
                    .raw(0, format!("Child {idx}"))
                    .styled(1, format!("{idx}"), id_style);
            }

            let columns = vec![
                table::Column::new(0, "Name", Constraint1D::new_min(10)),
                table::Column::new(1, "ID", Constraint1D::new(5, 10)),
            ];

            table::Table::new(&source_builder.finish())
                .id("tree")
                .columns(columns)
                .show_headers(true)
                .build(builder);
        });

        add_label(builder, "Press 'q' to quit");

        // Handle global events
        if builder.on_key_press(KeyCode::Char('q')) {
            running = false;
        }

        Ok(running)
    })?;

    Ok(())
}
