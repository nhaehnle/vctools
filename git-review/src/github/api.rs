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
