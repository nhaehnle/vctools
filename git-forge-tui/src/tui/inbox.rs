// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use vctuik::{
    layout::Constraint1D, prelude::*, state::Builder, table::{self, simple_table::{self, StyleId}}
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
        let read_style =
            table_builder.add_style(builder.theme().text(builder.theme_context()).inactive);
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
                    let mut repo_ids: HashMap<u64, u64> = HashMap::new();

                    for notification in notifications.into_iter() {
                        let repo_id = repo_ids
                            .entry(notification.repository.id)
                            .or_insert_with(|| {
                                table_builder
                                    .add(top_level, notification.repository.node_id.clone())
                                    .raw(
                                        0,
                                        format!(
                                            "{} / {}",
                                            &notification.repository.owner.login,
                                            &notification.repository.name
                                        ),
                                    )
                                    .id()
                            });
                        let style = if notification.unread {
                            StyleId::default()
                        } else {
                            read_style
                        };
                        let item =
                            table_builder
                            .add(*repo_id, notification.id.clone())
                            .styled(0, notification.subject.title.clone(), style)
                            .styled(1, notification.updated_at.clone(), style);
                        threads.insert(item.id(), (host, notification));
                    }
                }
                Err(err) => {
                    table_builder
                        .add(top_level, String::new())
                        .raw(0, err.to_string());
                }
            }
        }

        let columns = vec![
            table::Column::new(0, "", Constraint1D::unconstrained()),
            table::Column::new(1, "Last Update", Constraint1D::new(5, 20)),
        ];
        let selection = builder
            .nest()
            .id(state_id)
            .build(|builder| {
                table::Table::new(&table_builder.finish())
                    .id("tree")
                    .columns(columns)
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
