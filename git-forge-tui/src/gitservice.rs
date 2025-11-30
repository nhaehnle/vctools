// SPDX-License-Identifier: GPL-3.0-or-later

///! Managed asynchronous access to local git repositories.
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    hash::Hash,
    sync::{
        atomic::{self, AtomicBool},
        Arc, Condvar, Mutex, OnceLock,
    },
    time,
};

use blake2::Digest;
use log::{debug, error, info};
use serde::Deserialize;

use diff_modulo_base::git_core::{self, Cacheability, ExecutionProvider};
use vctools_utils::prelude::*;
use vctuik::signals::MergeWakeupSignal;

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
}
impl Default for Config {
    fn default() -> Self {
        Self {
            repositories: vec![],
        }
    }
}

#[derive(Debug, Default)]
struct RemotePrefetch {
    prefetched_remote: bool,
    prefetched_commits: HashSet<git_core::Ref>,
    pending: Vec<git_core::Ref>,
}

#[derive(Debug)]
struct Remote {
    git: GitRepository,
    api: ApiRepository,
    prefetch: Mutex<RemotePrefetch>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey([u8; 16]);

#[derive(Debug, Default)]
struct Cache {
    index: HashMap<CacheKey, git_core::ExecutionResult>,
}

#[derive(Debug)]
struct Job {
    key: CacheKey,
    path: std::path::PathBuf,
    command: String,
    args: Vec<OsString>,
}

#[derive(Debug)]
struct ServiceInner {
    repositories: Vec<git_core::Repository>,
    api_hosts: HashSet<String>,
    remotes: OnceLock<Vec<Remote>>,
    job: Mutex<Option<Job>>,
    job_available: Condvar,
    job_done: Condvar,
    cache: Mutex<Cache>,
    wakeup_signal: MergeWakeupSignal,
    timed_out: AtomicBool,
}

#[derive(Debug)]
pub struct GitService {
    inner: Arc<ServiceInner>,
    frame: Option<time::Instant>,
}
impl GitService {
    /// Create a new service with the provided configuration
    pub fn new(
        config: &Config,
        api_hosts: &[crate::github::Host],
        wakeup_signal: MergeWakeupSignal,
    ) -> Self {
        let repositories = config
            .repositories
            .iter()
            .map(|repo| git_core::Repository::new(repo.path.clone()))
            .collect();
        let api_hosts = api_hosts.iter().map(|host| host.host.clone()).collect();
        let inner = Arc::new(ServiceInner {
            repositories,
            api_hosts,
            remotes: OnceLock::new(),
            job: Mutex::new(None),
            job_available: Condvar::new(),
            job_done: Condvar::new(),
            cache: Mutex::new(Cache::default()),
            wakeup_signal,
            timed_out: AtomicBool::new(false),
        });

        {
            let inner_clone = Arc::clone(&inner);
            std::thread::spawn(move || {
                inner_clone.run_worker();
            });
        }

        Self { inner, frame: None }
    }

    pub fn start_frame(&mut self, frame_duration: time::Duration) {
        assert!(self.frame.is_none());
        self.frame = Some(time::Instant::now() + frame_duration);
        self.inner.timed_out.store(false, atomic::Ordering::Relaxed);
    }

    pub fn end_frame(&mut self) {
        assert!(self.frame.is_some());
        self.frame = None;
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
impl git_core::ExecutionProvider for GitService {
    fn exec(
        &self,
        path: &std::path::PathBuf,
        command: &str,
        args: Vec<std::ffi::OsString>,
        cacheable: Cacheability,
    ) -> git_core::ExecutionResult {
        let deadline = self.frame.unwrap(); // must be inside of a frame

        // Mutating commands are executed directly.
        if cacheable == Cacheability::None {
            return git_core::SimpleExecutionProvider.exec(path, command, args, cacheable);
        }

        type Blake2b128 = blake2::Blake2b<blake2::digest::consts::U16>;
        struct MyHasher(Blake2b128);
        impl std::hash::Hasher for MyHasher {
            fn finish(&self) -> u64 {
                unreachable!()
            }
            fn write(&mut self, bytes: &[u8]) {
                self.0.update(bytes);
            }
        }

        let mut hasher = MyHasher(Blake2b128::new());
        path.hash(&mut hasher);
        command.hash(&mut hasher);
        args.hash(&mut hasher);
        let res = hasher.0.finalize();
        let mut cache_key = CacheKey::default();
        cache_key.0.copy_from_slice(&res);

        // First, check if we have a cached result.
        let submitted = {
            let cache = self.inner.cache.lock().unwrap();
            if let Some(entry) = cache.index.get(&cache_key) {
                if !matches!(entry, git_core::ExecutionResult::Pending) {
                    return entry.clone();
                } else {
                    true
                }
            } else {
                false
            }
        };

        // Submit the job and wait for its completion.
        let mut job = self.inner.job.lock().unwrap();

        if !submitted {
            while job.is_some() {
                let timeout = deadline - time::Instant::now();
                if timeout.is_zero() {
                    self.inner.timed_out.store(true, atomic::Ordering::Relaxed);
                    return git_core::ExecutionResult::Pending;
                }
                job = self.inner.job_done.wait_timeout(job, timeout).unwrap().0;
            }

            *job = Some(Job {
                key: cache_key,
                path: path.clone(),
                command: command.to_string(),
                args,
            });

            let mut cache = self.inner.cache.lock().unwrap();
            cache
                .index
                .insert(cache_key, git_core::ExecutionResult::Pending);

            self.inner.job_available.notify_all();
        }

        loop {
            let timeout = deadline - time::Instant::now();
            if timeout.is_zero() {
                self.inner.timed_out.store(true, atomic::Ordering::Relaxed);
                return git_core::ExecutionResult::Pending;
            }
            job = self.inner.job_done.wait_timeout(job, timeout).unwrap().0;
            let cache = self.inner.cache.lock().unwrap();
            let entry = cache.index.get(&cache_key).unwrap();
            if !matches!(entry, git_core::ExecutionResult::Pending) {
                return entry.clone();
            }
        }
    }

    fn timed_out(&self) -> bool {
        assert!(self.frame.is_some());
        self.inner.timed_out.load(atomic::Ordering::Relaxed)
    }
}

impl ServiceInner {
    fn run_worker(&self) {
        // Initial collection of remotes.
        let mut remotes = Vec::new();
        for repo in &self.repositories {
            let current_remotes = match repo.get_remotes(&mut git_core::SimpleExecutionProvider) {
                Err(err) => {
                    error!(
                        "Error fetching remotes for {}: {}",
                        repo.path.display(),
                        err
                    );
                    continue;
                }
                Ok(remotes) => remotes,
            };
            for (remote, url) in current_remotes {
                let Some(hostname) = url.hostname() else {
                    continue;
                };
                let Some((owner, name)) = url.github_path() else {
                    continue;
                };

                if !self.api_hosts.contains(hostname) {
                    continue;
                }
                let api =
                    ApiRepository::new(hostname.to_string(), owner.to_string(), name.to_string());
                let git = GitRepository::new(repo.path.clone(), remote);
                remotes.push(Remote {
                    git,
                    api,
                    prefetch: Mutex::new(RemotePrefetch::default()),
                });
            }
        }
        self.remotes.set(remotes).unwrap();

        // Now go into the main worker loop
        let mut prefetch_grace = std::time::Instant::now();
        let mut prefetch_counter = 0;
        let mut job = self.job.lock().unwrap();
        'outer: loop {
            if let Some(the_job) = job.take() {
                self.job_done.notify_all();
                std::mem::drop(job);

                if let Err(err) = self.do_job(the_job) {
                    error!("gitservice error executing command: {}", err);
                }

                prefetch_grace = std::time::Instant::now() + std::time::Duration::from_secs(1);

                job = self.job.lock().unwrap();
                self.job_done.notify_all();
                if self.timed_out.load(atomic::Ordering::Relaxed) {
                    debug!("send wakeup signal");
                    self.wakeup_signal.signal();
                }
                continue;
            }

            // No command available. Wait for the grace period to pass.
            let duration = prefetch_grace - std::time::Instant::now();
            if !duration.is_zero() {
                job = self.job_available.wait_timeout(job, duration).unwrap().0;
                continue;
            }

            // Scan for prefetch requests.
            let remotes = self.remotes.wait();
            for i in 0..remotes.len() {
                let remote = &remotes[(prefetch_counter + i) % remotes.len()];
                let mut prefetches = remote.prefetch.lock().unwrap();

                if !prefetches.prefetched_remote {
                    prefetches.prefetched_remote = true;
                    std::mem::drop(prefetches);
                    std::mem::drop(job);

                    info!(
                        "Prefetching remote {} {}",
                        remote.git.repository.path.display(),
                        remote.git.remote
                    );
                    if let Err(err) = remote
                        .git
                        .repository
                        .prefetch(&git_core::SimpleExecutionProvider, &remote.git.remote)
                    {
                        error!(
                            "Error prefetching remote {} {}: {}",
                            remote.git.repository.path.display(),
                            remote.git.remote,
                            err
                        );
                    }

                    prefetch_counter = (prefetch_counter + i + 1) % remotes.len();
                    job = self.job.lock().unwrap();
                    continue 'outer;
                }

                if !prefetches.pending.is_empty() {
                    let pending = std::mem::take(&mut prefetches.pending);
                    prefetches
                        .prefetched_commits
                        .extend(pending.iter().cloned());
                    std::mem::drop(prefetches);
                    std::mem::drop(job);

                    if let Err(err) = remote.git.repository.fetch_missing(
                        &git_core::SimpleExecutionProvider,
                        &remote.git.remote,
                        &pending,
                    ) {
                        error!(
                            "Error prefetching commits for remote {} {}: {}",
                            remote.git.repository.path.display(),
                            remote.git.remote,
                            err
                        );
                    }

                    prefetch_counter = (prefetch_counter + i + 1) % remotes.len();
                    job = self.job.lock().unwrap();
                    continue 'outer;
                }
            }

            // No job and no prefetches. Just wait.
            assert!(job.is_none());
            job = self.job_available.wait(job).unwrap();
        }
    }

    fn do_job(&self, job: Job) -> Result<()> {
        debug!("Executing: git {} {:?}", job.command, job.args);
        let result = git_core::SimpleExecutionProvider.exec(
            &job.path,
            &job.command,
            job.args,
            Cacheability::None,
        );
        let mut cache = self.cache.lock().unwrap();
        *cache.index.get_mut(&job.key).unwrap() = result;
        Ok(())
    }
}
