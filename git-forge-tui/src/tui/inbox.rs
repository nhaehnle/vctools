// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use vctuik::{
    layout::Constraint1D, prelude::*, state::Builder, table::{self, simple_table}
};

use crate::github;

#[derive(Debug, Default)]
struct State {
    table_state: simple_table::SourceState<String>,
}

#[derive(Debug, Clone)]
pub struct InboxResult {
    /// Whether focus is on this widget
    pub has_focus: bool,

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
        let repo_style =
            table_builder.add_style(builder.theme().text(builder.theme_context()).header2);
        let mut threads: HashMap<u64, (&_, github::api::NotificationThread)> = HashMap::new();

        for (host, client) in connections.all_clients() {
            let top_level = table_builder
                .add(0, host.host.clone())
                .styled(0, &host.host, host_style)
                .id();

            let result =
                client.and_then(|client| {
                    let mut client = client.borrow_mut();
                    let notifications = client.access().notifications().ok()?;
                    Ok((client, notifications))
                });

            let Ok((mut client, notifications)) = result else {
                table_builder
                    .add(top_level, String::new())
                    .raw(0, result.err().unwrap().to_string());
                continue;
            };

            let prefetch = client.prefetch();
            let mut notifications =
                notifications
                    .into_iter()
                    .map(|n| {
                        let org = &n.repository.owner.login;
                        let gh_repo = &n.repository.name;
                        let pull =
                            n.pull_number().and_then(|id| {
                                prefetch
                                    .pull(org, gh_repo, id)
                                    .ok()
                                    .ok()
                            });
                        (n, pull)
                    })
                    .collect::<Vec<_>>();

            // We create table entries for repositories that have notifications
            // in alphabetical order.
            //
            // Map API repo IDs to repo table item IDs.
            let repo_ids = {
                let mut repo_ids: HashMap<u64, u64> = HashMap::new();
                let mut repos = Vec::new();
                for (n, _) in &notifications {
                    repo_ids.entry(n.repository.id)
                        .or_insert_with(|| {
                            repos.push(&n.repository);
                            0
                        });
                }
                repos.sort_by(|a, b| {
                    let ord = a.owner.login.cmp(&b.owner.login);
                    if ord == std::cmp::Ordering::Equal {
                        a.name.cmp(&b.name)
                    } else {
                        ord
                    }
                });
                for repo in repos {
                    let id = table_builder
                        .add(top_level, repo.node_id.clone())
                            .styled(
                                0,
                                format!(
                                    "{} / {}",
                                    &repo.owner.login,
                                    &repo.name
                                ),
                                repo_style,
                            )
                            .id();
                    *repo_ids.get_mut(&repo.id).unwrap() = id;
                }
                repo_ids
            };

            // We determine parent-child relationships between notifications
            // for stacked pull requests.
            //
            // TODO: Ideally, we'd also show read pull requests of a stack here
            //       in some grayed-out way. But that requires a different
            //       approach -- maintaining our own cache database of pull
            //       requests that we can efficiently query by branch name.
            //
            // Map of (repo ID, head ref) to (notification table item IDs, # children).
            let mut head_map: HashMap<(u64, &str), (Option<u64>, usize)> = HashMap::new();

            for (_, pull) in &notifications {
                let Some(pull) = pull else { continue };

                let repo_id = pull.head.repo.id;
                let head_ref = pull.head.ref_.as_str();
                head_map.insert((repo_id, head_ref), (None, 0));
            }

            for (_, pull) in &notifications {
                let Some(pull) = pull else { continue };

                let repo_id = pull.base.repo.id;
                let head_ref = pull.base.ref_.as_str();
                if let Some((_, count)) = head_map.get_mut(&(repo_id, head_ref)) {
                    *count += 1;
                }
            }

            // Now iterate multiple times to create parent items before child items.
            let mut worklist = notifications.iter().enumerate().collect::<Vec<_>>();
            let mut item_ids = Vec::new();
            while !worklist.is_empty() {
                for workitem in std::mem::take(&mut worklist) {
                    let (notification_idx, (notification, pull)) = workitem;

                    // Determine the parent item in the table.
                    let mut parent_id = None;
                    if let Some(pull) = pull {
                        let base_repo_id = pull.base.repo.id;
                        let base_ref = pull.base.ref_.as_str();
                        if let Some((parent, _)) = head_map.get(&(base_repo_id, base_ref)) {
                            if parent.is_none() {
                                worklist.push(workitem);
                                continue;
                            }
                            parent_id = *parent;
                        }
                    }
                    let is_top_level_in_repo = parent_id.is_none();
                    let parent_id =
                        parent_id.unwrap_or_else(|| {
                            *repo_ids.get(&notification.repository.id).unwrap()
                        });

                    // Create the table item for this notification.
                    let item =
                        table_builder
                        .add(parent_id, notification.id.clone())
                        .raw(0, notification.subject.title.clone())
                        .raw(1, notification.updated_at.clone());
                    let item_id = item.id();

                    if let Some(pull) = pull {
                        let repo_id = pull.head.repo.id;
                        let head_ref = pull.head.ref_.as_str();
                        let entry = head_map.get_mut(&(repo_id, head_ref)).unwrap();
                        if entry.1 <= 1 && !is_top_level_in_repo {
                            entry.0 = Some(parent_id);
                        } else {
                            entry.0 = Some(item_id);
                        }
                    }

                    item_ids.push((notification_idx, item_id));
                }
            }

            for (notification_idx, item_id) in item_ids {
                threads.insert(item_id, (host, std::mem::take(&mut notifications[notification_idx].0)));
            }
        }

        let columns = vec![
            table::Column::new(0, "", Constraint1D::unconstrained()),
            table::Column::new(1, "Last Update", Constraint1D::new(5, 20)),
        ];
        let table_result = builder
            .nest()
            .id(state_id)
            .build(|builder| {
                table::Table::new(&table_builder.finish())
                    .id("tree")
                    .columns(columns)
                    .build(builder)
            });

        InboxResult {
            has_focus: table_result.has_focus,
            selection: table_result
                .selection
                .and_then(|id| threads.remove(&id))
                .map(|(host, thread)| (host.host.clone(), thread)),
        }
    }
}
