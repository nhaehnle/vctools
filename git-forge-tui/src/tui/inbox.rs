// SPDX-License-Identifier: GPL-3.0-or-later

use vctuik::{prelude::*, state::Builder, table::{self, simple_table}};

use crate::github;

#[derive(Debug, Default)]
struct State {
    table_state: simple_table::SourceState<String>,
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

        let mut table_builder = state.table_state.build();

        let host_style = table_builder.add_style(builder.theme().text(builder.theme_context()).header1);

        for (host, client) in connections.all_clients() {
            let top_level =
                table_builder.add(0, host.host.clone())
                    .styled(0, &host.host, host_style)
                    .id();

            let response =
                client.and_then(|client| {
                    Ok(client.borrow_mut().access().notifications().ok()?)
                });
            match response {
            Ok(notifications) => {
                for notification in notifications.into_iter() {
                    table_builder.add(top_level, notification.id)
                        .raw(0, notification.subject.title);
                }
            },
            Err(err) => {
                table_builder.add(top_level, String::new())
                    .raw(0, err.to_string());
            },
            }
        }

        builder.nest().id(state_id).build(|builder| {
            table::Table::new(&table_builder.finish())
                .id("tree")
                .build(builder);
        });
    }
}
