use std::{
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};
use reqwest::{blocking::Client, header};
use serde::Deserialize;

use crate::model::{
    ForgeTrait,
};

#[derive(Clone, Debug, Deserialize)]
pub struct GitHubAccount {
    url: String,
    user: String,
    token: String,
}

#[derive(Debug)]
struct GitHubInner {
    account: GitHubAccount,
}

enum Command {
    Exit,
}

struct RateLimit {
    duration: Duration,
    until: Instant,
}

struct GitHubWorker {
    inner: Arc<GitHubInner>,
    done: bool,
    next_poll: Option<Instant>,
    client: Client,
    rate_limit: Option<RateLimit>,
    notifications_last_modified: Option<String>,
}
impl GitHubWorker {
    fn new(inner: Arc<GitHubInner>) -> Self {
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
            done: false,
            next_poll: Some(Instant::now()),
            client,
            rate_limit: None,
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
                if response.status().is_success() {
                    self.notifications_last_modified = response.headers().get(header::LAST_MODIFIED)
                            .and_then(|v| v.to_str().ok()).map(|s| s.to_string());

                    //let notifications: serde_json::Value = response.json().unwrap();
                } else if response.status() == reqwest::StatusCode::NOT_MODIFIED {
                    log::info!("GitHub notifications not modified");
                } else {
                    log::error!("Failed to fetch notifications: {:?}", response.status());
                    return;
                }

                if let Some(seconds) = response.headers().get("X-Poll-Interval")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok()) {
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
    pub fn open(account: GitHubAccount) -> Self {
        let inner = Arc::new(GitHubInner {
            account,
        });

        let (send, recv) = mpsc::channel();

        let inner_clone = inner.clone();
        let thread_join = thread::spawn(move || GitHubWorker::new(inner_clone).run(recv));

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
}
