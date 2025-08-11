// SPDX-License-Identifier: GPL-3.0-or-later

use std::cell::RefCell;

use ratatui::text::Text;
use vctuik::{layout::{Constraint1D, LayoutItem1D}, prelude::*, state::{self, Builder}, tree::{Tree, TreeBuild, TreeItem}};

use crate::github::{self, api, Client};

#[derive(Debug, Default)]
struct State {
}

#[derive(Debug)]
pub struct Inbox {

}
impl Inbox {
    pub fn new() -> Self {
        Self {}
    }

    pub fn build(self, builder: &mut Builder, connections: &mut github::connections::Connections) {
        let state_id = builder.add_state_id("inbox");
        let state: &mut State = builder.get_state(state_id);

        let mut top_level = Vec::new();

        for (idx, (host, client)) in connections.all_clients().enumerate() {
            let mut children = Vec::new();
            let response =
                client.and_then(|client| {
                    Ok(client.borrow_mut().access().notifications().ok()?)
                });
            match response {
            Ok(notifications) => {
                for (idx, notification) in notifications.into_iter().enumerate() {
                    children.push(TreeItem::new_leaf(idx, notification.subject.title))
                }
            },
            Err(err) => {
                children.push(TreeItem::new_leaf(usize::MAX, err.to_string()))
            },
            }
            top_level.push(TreeItem::new(idx, host.host.clone(), children).unwrap());
        }

        builder.nest().id(state_id).build(|builder| {
            Tree::new(&top_level).unwrap()
                .build(builder, "tree");
        });
    }
}
