// SPDX-License-Identifier: GPL-3.0-or-later

///! Managed asynchronous access to local git repositories.

use std::{collections::HashSet, sync::{atomic::{self, AtomicUsize}, Arc, Condvar, Mutex, OnceLock}};

use log::{info, error};
use serde::Deserialize;

use diff_modulo_base::git_core;
use vctools_utils::prelude::*;

use crate::{ApiRepository, GitRepository};

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RepositoryConfig {
    pub path: std::path::PathBuf,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    #[serde(rename = "repository")]
    pub repositories: Vec<RepositoryConfig>,

    pub num_workers: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            repositories: vec![],
            num_workers: 4,
        }
    }
}

#[derive(Debug)]
struct Remote {
    git: GitRepository,
    api: ApiRepository,
}

#[derive(Debug)]
enum Job {
    PrefetchRemote(usize),
}

#[derive(Debug)]
struct ServiceInner {
    repositories: Vec<git_core::Repository>,
    api_hosts: HashSet<String>,
    init_job_count: AtomicUsize,
    remotes_collect: Mutex<(usize, Vec<Remote>)>,
    remotes: OnceLock<Vec<Remote>>,
    job_available: Condvar,
    jobs: Mutex<Vec<Job>>,
}

#[derive(Debug)]
pub struct GitService {
    inner: Arc<ServiceInner>,
}
impl GitService {
    /// Create a new service with the provided configuration
    pub fn new(config: &Config, api_hosts: &[crate::github::Host]) -> Self {
        let repositories = config.repositories.iter()
            .map(|repo| git_core::Repository::new(repo.path.clone()))
            .collect();
        let api_hosts = api_hosts.iter().map(|host| host.host.clone()).collect();
        let inner = Arc::new(ServiceInner {
            repositories,
            api_hosts,
            init_job_count: AtomicUsize::new(0),
            remotes_collect: Mutex::new((0, vec![])),
            remotes: OnceLock::new(),
            job_available: Condvar::new(),
            jobs: Mutex::new(vec![]),
        });

        for _ in 0..config.num_workers {
            let inner_clone = Arc::clone(&inner);
            std::thread::spawn(move || {
                inner_clone.run_worker();
            });
        }

        Self {
            inner,
        }
    }

    pub fn find_git(&self, api: &ApiRepository) -> Option<&GitRepository> {
        let remotes = self.inner.remotes.wait();

        remotes.iter().find_map(|remote| {
            if remote.api == *api {
                Some(&remote.git)
            } else {
                None
            }
        })
    }
}

impl ServiceInner {
    fn run_worker(&self) {
        // Read all remotes
        loop {
            let init_job = self.init_job_count.fetch_add(1, atomic::Ordering::Relaxed);
            if init_job >= self.repositories.len() {
                break;
            }

            let repo = &self.repositories[init_job];
            let mut current_remotes =
                match repo.get_remotes() {
                    Err(err) => {
                        error!("Error fetching remotes for {}: {}", repo.path.display(), err);
                        continue;
                    },
                    Ok(remotes) => {
                        remotes.into_iter()
                            .filter_map(|(remote, url)| {
                                url.hostname().zip(url.github_path())
                                    .filter(|&(hostname, _)| self.api_hosts.contains(hostname))
                                    .map(|(hostname, (owner, name))| {
                                        let api = ApiRepository::new(
                                            hostname.to_string(),
                                            owner.to_string(),
                                            name.to_string());
                                        let git = GitRepository::new(repo.path.clone(), remote);
                                        Remote { git, api }
                                    })
                            })
                            .collect::<Vec<_>>()
                    },
                };

            let mut remotes = self.remotes_collect.lock().unwrap();

            remotes.1.append(&mut current_remotes);

            remotes.0 += 1;
            if remotes.0 >= self.repositories.len() {
                // All remotes collected, get ready for the main thread.
                let num_remotes = remotes.1.len();
                self.remotes.set(std::mem::take(&mut remotes.1)).unwrap();
                std::mem::drop(remotes);

                let mut jobs = self.jobs.lock().unwrap();
                jobs.extend((0..num_remotes).rev().map(Job::PrefetchRemote));
                self.job_available.notify_all();
                break;
            }
        }

        // Main job loop
        let mut jobs = self.jobs.lock().unwrap();
        loop {
            let Some(job) = jobs.pop() else {
                jobs = self.job_available.wait(jobs).unwrap();
                continue;
            };

            std::mem::drop(jobs);

            if let Err(err) = self.do_job(job) {
                error!("gitservice error: {}", err);
            }

            jobs = self.jobs.lock().unwrap();
        }
    }

    fn do_job(&self, job: Job) -> Result<()> {
        match job {
            Job::PrefetchRemote(remote_index) => {
                let remotes = self.remotes.wait();
                let remote = &remotes[remote_index];

                info!("Prefetching {:?}", remote);

                remote.git.repository.prefetch(&remote.git.remote)?;
            },
        }

        Ok(())
    }
}
