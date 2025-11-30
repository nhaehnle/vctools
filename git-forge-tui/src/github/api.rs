// SPDX-License-Identifier: GPL-3.0-or-later

use serde::Deserialize;

#[derive(Deserialize, Default, Debug, Clone)]
pub struct Branch {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub sha: String,
    pub repo: MinimalRepository,
}

#[derive(Deserialize, Default, Debug, Clone)]
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
impl Default for PullState {
    fn default() -> Self {
        PullState::Other
    }
}

#[derive(Deserialize, Default, Debug, Clone)]
pub struct Pull {
    pub number: u64,
    pub state: PullState,
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
    pub html_url: String,
}

#[derive(Deserialize, Default, Debug, Clone)]
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
impl Default for ReviewState {
    fn default() -> Self {
        ReviewState::Other
    }
}

#[derive(Deserialize, Default, Debug, Clone)]
pub struct Review {
    pub user: User,

    // The Copilot pull request reviewer bot creates reviews without a commit ID.
    pub commit_id: Option<String>,
    pub submitted_at: String,
    pub body: String,
    pub state: ReviewState,
}

#[derive(Deserialize, Default, Debug, Clone)]
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
impl Default for SubjectType {
    fn default() -> Self {
        SubjectType::Unknown
    }
}

#[derive(Deserialize, Default, Debug, Clone)]
pub struct NotificationSubject {
    pub title: String,
    pub url: String,
    #[serde(rename = "type")]
    pub subject_type: SubjectType,
}

#[derive(Deserialize, Default, Debug, Clone)]
pub struct NotificationThread {
    pub id: String,
    pub last_read_at: Option<String>,
    pub reason: String,
    pub repository: MinimalRepository,
    pub subject: NotificationSubject,
    pub updated_at: String,
}
impl NotificationThread {
    pub fn updated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        match chrono::DateTime::parse_from_rfc3339(&self.updated_at) {
            Ok(dt) => Some(dt.with_timezone(&chrono::Utc)),
            Err(_) => None,
        }
    }

    pub fn pull_number(&self) -> Option<u64> {
        if self.subject.subject_type != SubjectType::PullRequest {
            return None;
        }
        self
            .subject
            .url
            .split('/')
            .last()
            .and_then(|id_str| id_str.parse::<u64>().ok())
    }
}
