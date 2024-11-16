use std::cell::RefCell;

use tui_tree_widget::{Tree, TreeItem};

use vctuik::{
    check_box::add_check_box,
    event::{self, KeyCode},
    label::add_label,
    panes::{Pane, Panes},
    prelude::*,
    tree::TreeBuild,
};

fn main() -> Result<()> {
    let mut terminal = vctuik::init()?;

    let mut running = true;

    let mut foo = false;
    let mut bar = false;

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
        let running = RefCell::new(&mut running);
        terminal.run_frame(|builder| {
            let mut panes = Panes::new();
            panes.add(Pane::new("Settings"), |builder| {
                add_check_box(builder, "Foo", &mut foo);
                add_check_box(builder, "Bar", &mut bar);
            });
            panes.add(Pane::new("Commentary"), |builder| {
                add_label(builder, "Cheddar");
                add_label(builder, "Provolone");
                add_label(builder, "Swiss");
            });
            panes.add(Pane::new("Running"), |builder| {
                add_check_box(builder, "Running", &running);
            });
            panes.add(Pane::new("Tree"), |builder| {
                Tree::new(&items).unwrap()
                    .build(builder, "tree", u16::MAX);
            });
            panes.build(builder, "panes", builder.viewport().height - 1);
            event::on_key_press(builder, KeyCode::Char('q'), |_| {
                **running.borrow_mut() = false;
            });
        })?;
    }

    Ok(())
}
