// SPDX-License-Identifier: GPL-3.0-or-later

mod config;
pub mod github;
pub mod gitservice;
pub mod logview;
pub mod tui;

pub use config::{get_project_dirs, load_config};

use diff_modulo_base::git_core;
use vctools_utils::prelude::*;

/// Reference to a forge repository through a local clone and a remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepository {
    pub repository: git_core::Repository,
    pub remote: String,
}
impl GitRepository {
    pub fn new(path: std::path::PathBuf, remote: String) -> Self {
        Self {
            repository: git_core::Repository::new(path),
            remote,
        }
    }
}

/// Reference to a forge repository through its API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}
impl ApiRepository {
    pub fn new(host: String, owner: String, name: String) -> Self {
        Self { host, owner, name }
    }
}

/// Depending on the context, a pull request can be identified through a local
/// clone and remote, or through the API endpoint, or both.
///
/// We always need the ID of the pull request, which is used to identify it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    git: Option<GitRepository>,
    api: Option<ApiRepository>,
    id: u64,
}
impl PullRequest {
    pub fn new(git: GitRepository, api: ApiRepository, id: u64) -> Self {
        Self {
            git: Some(git),
            api: Some(api),
            id,
        }
    }

    pub fn from_git(git: GitRepository, id: u64) -> Self {
        Self {
            git: Some(git),
            api: None,
            id,
        }
    }

    pub fn from_api(api: ApiRepository, id: u64) -> Self {
        Self {
            git: None,
            api: Some(api),
            id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletePullRequest {
    git: GitRepository,
    api: ApiRepository,
    id: u64,
}
impl CompletePullRequest {
    pub fn from_git(
        git: GitRepository,
        id: u64,
        hosts: &[github::Host],
        ep: &dyn git_core::ExecutionProvider,
    ) -> Result<Self> {
        let url = git.repository.get_url(ep, &git.remote)?;

        let Some(hostname) = url.hostname() else {
            Err(format!("cannot find hostname for {url}"))?
        };
        let Some((owner, name)) = url.github_path() else {
            Err(format!("cannot parse {url} as a GitHub repository"))?
        };

        let Some(host) = hosts.iter().find(|host| host.matches_host(hostname)) else {
            Err(format!(
                "Host not configured; add it to your github.toml: {hostname}"
            ))?
        };

        let api = ApiRepository::new(host.host.to_string(), owner.to_string(), name.to_string());
        Ok(CompletePullRequest { git, api, id })
    }

    pub fn from_api(
        api: ApiRepository,
        id: u64,
        git_service: &gitservice::GitService,
    ) -> Result<Self> {
        let Some(git) = git_service.find_git(&api) else {
            Err(format!(
                "Local clone not found; add it to your repositories.toml: {}/{}",
                api.owner, api.name
            ))?
        };
        Ok(CompletePullRequest {
            git: git.clone(),
            api,
            id,
        })
    }
}
