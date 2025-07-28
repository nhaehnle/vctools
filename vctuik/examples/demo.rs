use ratatui::widgets::Block;
use tui_tree_widget::{Tree, TreeItem};

use vctuik::{
    check_box::add_check_box, section::with_section, event::KeyCode, label::add_label,
    input::Input,
    prelude::*,
    tree::TreeBuild,
};

fn main() -> Result<()> {
    let mut terminal = vctuik::init()?;

    let mut running = true;

    let mut foo = false;
    let mut bar = false;
    let mut name: String = "world".into();

    let items = vec![
        TreeItem::new(0, "Root", vec![
            TreeItem::new_leaf(0, "Child 1"),
            TreeItem::new_leaf(1, "Child 2"),
            TreeItem::new(2, "Child 3", vec![
                TreeItem::new_leaf(0, "Grandchild 1"),
                TreeItem::new_leaf(1, "Grandchild 2"),
            ]).unwrap(),
        ]).unwrap()
    ];

    while running {
        while terminal.run_frame(|builder| {
            // Clear the window
            let frame_area = builder.frame().area();
            let block = Block::new()
                .style(builder.theme().pane_background);
            builder.frame().render_widget(block, frame_area);

            // Draw UI
            with_section(builder, "Settings", |builder| {
                add_check_box(builder, "Foo", &mut foo);
                add_check_box(builder, "Bar", &mut bar);
                Input::new("name")
                    .label("Name:")
                    .build(builder, &mut name);
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
                builder.add_slack();
            });

            with_section(builder, "Tree", |builder| {
               Tree::new(&items).unwrap()
                   .build(builder, "tree");
            });

            add_label(builder, "Press 'q' to quit");

            // Handle global events
            if builder.on_key_press(KeyCode::Char('q')) {
                running = false;
                return;
            }
        })? {
            // repeat until settled
        }
    }

    Ok(())
}
