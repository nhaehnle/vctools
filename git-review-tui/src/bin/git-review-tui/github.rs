use std::{
    sync::{mpsc, Arc, RwLock},
    thread,
    time::{Duration, Instant},
};
use reqwest::{blocking::Client, header};
use serde::Deserialize;

use crate::model::{
    ForgeTrait,
    Repository,
};

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubAccount {
    url: String,
    user: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct ApiSimpleOwner {
    login: String,
    node_id: String,
}

#[derive(Debug, Deserialize)]
struct ApiMinimalRepository {
    id: u64,
    node_id: String,
    name: String,
    owner: ApiSimpleOwner,
}

#[derive(Debug, Deserialize)]
struct ApiNotificationSubject {
    title: String,
    url: String,
    #[serde(rename = "type")]
    subject_type: String,
}

#[derive(Debug, Deserialize)]
struct ApiNotificationThread {
    id: String,
    last_read_at: Option<String>,
    reason: String,
    repository: ApiMinimalRepository,
    subject: ApiNotificationSubject,
    unread: bool,
    updated_at: String,
}

#[derive(Debug)]
struct GitHubRepository {
    id: usize,
    owner: String,
    name: String,
}

#[derive(Debug)]
struct GitHubRepositories {
    repositories: Vec<GitHubRepository>,
}
impl GitHubRepositories {
    fn new() -> Self {
        Self {
            repositories: Vec::new(),
        }
    }

    fn get_or_insert(&mut self, owner: &str, name: &str) -> &mut GitHubRepository {
        let id = self.repositories.iter()
            .find(|r| r.owner == owner && r.name == name)
            .map(|r| r.id)
            .unwrap_or_else(|| {
                let id = self.repositories.len();
                self.repositories.push(GitHubRepository {
                    id,
                    owner: owner.to_string(),
                    name: name.to_string(),
                });
                id
            });
        &mut self.repositories[id]
    }
}

#[derive(Debug)]
struct GitHubInner {
    account: GitHubAccount,
    repositories: RwLock<GitHubRepositories>,
}

enum Command {
    Exit,
}

trait Notify {
    fn call(&self) -> bool;
}

struct GitHubWorker {
    inner: Arc<GitHubInner>,
    notify: Box<dyn Notify>,
    done: bool,
    next_poll: Option<Instant>,
    client: Client,
    notifications_last_modified: Option<String>,
}
impl GitHubWorker {
    fn new(inner: Arc<GitHubInner>, notify: Box<dyn Notify>) -> Self {
        use reqwest::header;

        let mut headers = header::HeaderMap::new();
        headers.insert("X-GitHub-Api-Version", "2022-11-28".try_into().unwrap());
        headers.insert(header::ACCEPT, "application/vnd.github+json".try_into().unwrap());

        let mut auth: header::HeaderValue = format!("Bearer {}", inner.account.token).try_into().unwrap();
        auth.set_sensitive(true);
        headers.insert(header::AUTHORIZATION, auth);

        headers.insert(header::USER_AGENT, "git-review-tui".try_into().unwrap());

        let client = Client::builder()
            .default_headers(headers)
            .build().unwrap();

        Self {
            inner,
            notify,
            done: false,
            next_poll: Some(Instant::now()),
            client,
            notifications_last_modified: None,
        }
    }

   fn run(&mut self, recv: mpsc::Receiver<Command>) {
        log::info!("GitHub forge started for account {:?}", &self.inner.account);

        while !self.done {
            match recv.try_recv() {
                Ok(command) => {
                    self.process(command);
                    continue;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    log::info!("GitHub forge worker command connection broken");
                    break;
                }
            };

            if let Some(next_poll) = self.next_poll {
                let Some(timeout) = next_poll.checked_duration_since(Instant::now()) else {
                    self.poll();
                    continue;
                };

                match recv.recv_timeout(timeout) {
                    Ok(command) => self.process(command),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        self.poll();
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        log::info!("GitHub forge worker command connection broken");
                        break;
                    }
                };
            } else {
                match recv.recv() {
                    Ok(command) => self.process(command),
                    Err(_) => {
                        log::info!("GitHub forge worker command connection broken");
                        break;
                    }
                };
            }
        }
    }

    fn process(&mut self, command: Command) {
        match command {
            Command::Exit => {
                self.done = true;
            }
        }
    }

    fn poll(&mut self) {
        log::info!("GitHub forge worker polling");
        self.next_poll = None;

        // Query notifications to find updated code reviews
        let mut request = self.client.get(format!("{}/notifications", self.inner.account.url))
                .query(&[("all", "true")]);
        if let Some(last_modified) = &self.notifications_last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, last_modified);
        }
        log::info!("Request: {:?}", &request);
        match request.send() {
            Err(err) => {
                log::error!("Failed to fetch notifications: {:?}", err);
            }
            Ok(response) => {
                log::info!("GitHub notifications: {:?}", &response);

                let poll_interval = response.headers().get("X-Poll-Interval")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok());

                if response.status().is_success() {
                    self.notifications_last_modified = response.headers().get(header::LAST_MODIFIED)
                            .and_then(|v| v.to_str().ok()).map(|s| s.to_string());

                    let notifications: Vec<ApiNotificationThread> =
                        match response.json() {
                            Ok(notifications) => notifications,
                            Err(err) => {
                                log::error!("Failed to parse notifications: {:?}", err);
                                return;
                            }
                        };

                    let mut repositories = self.inner.repositories.write().unwrap();
                    for thread in notifications {
                        log::info!("Notification: {:?}", &thread);
                        repositories.get_or_insert(&thread.repository.owner.login, &thread.repository.name);
                    }

                    if !self.notify.call() {
                        self.done = true;
                    }
                } else if response.status() == reqwest::StatusCode::NOT_MODIFIED {
                    log::info!("GitHub notifications not modified");
                } else {
                    log::error!("Failed to fetch notifications: {:?}", response.status());
                    return;
                }

                if  let Some(seconds) = poll_interval {
                    self.next_poll = Some(Instant::now() + Duration::from_secs(seconds));
                }
            }
        }
    }
}

#[derive(Debug)]
struct GitHubControl {
    send: mpsc::Sender<Command>,
    thread_join: thread::JoinHandle<()>,
}

#[derive(Debug)]
pub struct GitHubForge {
    inner: Arc<GitHubInner>,
    control: GitHubControl,
}
impl GitHubForge {
    pub fn open<N: Fn() -> bool + Send + 'static>(account: GitHubAccount, notify: N) -> Self {
        let inner = Arc::new(GitHubInner {
            account,
            repositories: RwLock::new(GitHubRepositories::new()),
        });

        let (send, recv) = mpsc::channel();

        let inner_clone = inner.clone();

        struct NotifyImpl<F> {
            f: F,
        }
        impl<F> Notify for NotifyImpl<F>
            where F: Fn() -> bool
        {
            fn call(&self) -> bool {
                (self.f)()
            }
        }
        let notify = Box::new(NotifyImpl { f: notify });

        let thread_join = thread::spawn(
                move || GitHubWorker::new(inner_clone, notify).run(recv));

        Self {
            inner,
            control: GitHubControl {
                send,
                thread_join,
            },
        }
    }

    pub fn close(self) {
        let _ = self.control.send.send(Command::Exit);
        self.control.thread_join.join().unwrap();
    }
}
impl ForgeTrait for GitHubForge {
    fn get_repositories(&self) -> Vec<Repository> {
        let repositories = self.inner.repositories.read().unwrap();
        repositories.repositories.iter().map(|r| Repository {
            id: r.id,
            name: vec![r.owner.clone(), r.name.clone()],
            code_reviews: Vec::new(),
        }).collect()
    }
}
