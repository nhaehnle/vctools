// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use vctuik::{
    prelude::*,
    state::Builder,
    table::{self, simple_table},
};

use crate::github;

#[derive(Debug, Default)]
struct State {
    table_state: simple_table::SourceState<String>,
}

#[derive(Debug, Clone)]
pub struct InboxResult {
    /// Host and notification thread of the current selection
    pub selection: Option<(String, github::api::NotificationThread)>,
}

#[derive(Debug)]
pub struct Inbox {}
impl Inbox {
    pub fn new() -> Self {
        Self {}
    }

    pub fn build(
        self,
        builder: &mut Builder,
        connections: &mut github::connections::Connections,
    ) -> InboxResult {
        let state_id = builder.add_state_id("inbox");
        let state: &mut State = builder.get_state(state_id);

        let mut table_builder = state.table_state.build();

        let host_style =
            table_builder.add_style(builder.theme().text(builder.theme_context()).header1);
        let mut threads: HashMap<u64, (&_, github::api::NotificationThread)> = HashMap::new();

        for (host, client) in connections.all_clients() {
            let top_level = table_builder
                .add(0, host.host.clone())
                .styled(0, &host.host, host_style)
                .id();

            let response =
                client.and_then(|client| Ok(client.borrow_mut().access().notifications().ok()?));
            match response {
                Ok(notifications) => {
                    for notification in notifications.into_iter() {
                        let id = table_builder
                            .add(top_level, notification.id.clone())
                            .raw(0, notification.subject.title.clone())
                            .id();
                        threads.insert(id, (host, notification));
                    }
                }
                Err(err) => {
                    table_builder
                        .add(top_level, String::new())
                        .raw(0, err.to_string());
                }
            }
        }

        let selection = builder
            .nest()
            .id(state_id)
            .build(|builder| {
                table::Table::new(&table_builder.finish())
                    .id("tree")
                    .build(builder)
            })
            .selection;

        InboxResult {
            selection: selection
                .and_then(|id| threads.remove(&id))
                .map(|(host, thread)| (host.host.clone(), thread)),
        }
    }
}
