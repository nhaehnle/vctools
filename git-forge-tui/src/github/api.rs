// SPDX-License-Identifier: GPL-3.0-or-later

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Branch {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub sha: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Pull {
    pub head: Branch,
    pub base: Branch,
}

#[derive(Deserialize, Debug, Clone)]
pub struct User {
    pub login: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Review {
    pub user: User,
    pub commit_id: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MinimalRepository {
    pub id: u64,
    pub node_id: String,
    pub name: String,
    pub owner: User,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum SubjectType {
    Issue,
    PullRequest,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NotificationSubject {
    pub title: String,
    pub url: String,
    #[serde(rename = "type")]
    pub subject_type: SubjectType,
}

#[derive(Deserialize, Debug, Clone)]
pub struct NotificationThread {
    pub id: String,
    pub last_read_at: Option<String>,
    pub reason: String,
    pub repository: MinimalRepository,
    pub subject: NotificationSubject,
    pub unread: bool,
    pub updated_at: String,
}
