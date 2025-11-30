// SPDX-License-Identifier: GPL-3.0-or-later

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Branch {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub sha: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Label {
    pub name: String,
}

#[derive(Deserialize, Debug, Clone)]
pub enum PullState {
    #[serde(rename = "open")]
    Open,
    #[serde(rename = "closed")]
    Closed,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Pull {
    pub number: u64,
    pub state: String,
    pub draft: bool,
    pub merged: bool,
    pub user: User,
    pub head: Branch,
    pub base: Branch,
    pub title: String,
    pub body: Option<String>,
    pub labels: Vec<Label>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
    pub assignees: Vec<User>,
    pub requested_reviewers: Vec<User>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct User {
    pub login: String,
}

#[derive(Deserialize, Debug, Clone)]
pub enum ReviewState {
    #[serde(rename = "APPROVED")]
    Approved,
    #[serde(rename = "CHANGES_REQUESTED")]
    ChangesRequested,
    #[serde(rename = "COMMENTED")]
    Commented,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Review {
    pub user: User,
    pub commit_id: String,
    pub submitted_at: String,
    pub body: String,
    pub state: ReviewState,
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
impl NotificationThread {
    pub fn updated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        match chrono::DateTime::parse_from_rfc3339(&self.updated_at) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(_) => None,
        }
    }
}
