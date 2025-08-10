// SPDX-License-Identifier: GPL-3.0-or-later

use std::{any::Any, borrow::Cow, collections::HashMap, ops::DerefMut, sync::{Arc, Condvar, Mutex, MutexGuard}, thread::JoinHandle, time::{Duration, Instant}};

use itertools::Itertools;
use log::{trace, debug, info, warn, error, LevelFilter};
use reqwest::{header, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize};
use vctools_utils::prelude::*;

pub mod api;

#[derive(Deserialize, Debug, Clone)]
pub struct Host {
    pub host: String,
    pub api: String,
    pub user: String,
    pub token: String,
}

#[derive(Debug)]
pub struct ClientConfig {
    host: Host,
    offline: bool,
}
impl ClientConfig {
    pub fn offline(self) -> Self {
        Self {
            offline: true,
            ..self
        }
    }

    pub fn build(self) -> Result<Client> {
        let url_api = Url::parse(&self.host.api)?;
        let mut client = Client {
            config: self,
            url_api,
            cache: Arc::new(Cache::default()),
            helper: None,
        };

        if !client.config.offline {
            client.start_thread()?;
        }

        Ok(client)
    }
}

#[derive(Debug)]
pub struct Client {
    config: ClientConfig,
    url_api: Url,
    cache: Arc<Cache>,
    helper: Option<Arc<HelperCtrl>>,
}
impl Client {
    pub fn build(host: Host) -> ClientConfig {
        ClientConfig {
            host,
            offline: false,
        }
    }

    fn start_thread(&mut self) -> Result<()> {
        let helper = Arc::new(HelperCtrl {
            response_notify: Condvar::new(),
            helper_wakeup: Condvar::new(),
            state: Mutex::new(HelperState {
                running: true,
                frame_requests: Vec::new(),
                backlog_requests: Vec::new(),
            }),
        });
        self.helper = Some(helper.clone());

        let cache = self.cache.clone();
        let host = self.config.host.clone();
        let url_api = self.url_api.clone();

        std::thread::spawn(move || {
            run_helper(
                cache,
                helper,
                host,
                url_api,
            );
        });

        Ok(())
    }

    pub fn frame(&mut self, max_duration: Option<Duration>) -> ClientFrame {
        let wait_policy =
            if max_duration.is_some() {
                WaitPolicy::Deadline(Instant::now() + max_duration.unwrap())
            } else {
                WaitPolicy::Wait
            };

        ClientFrame {
            client: self,
            wait_policy,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum WaitPolicy {
    Wait,
    Deadline(Instant),
    Prefetch,
}

trait DynParser: std::fmt::Debug + Send + Sync {
    fn parse(&self, s: &str) -> Result<Box<dyn Any + Send + Sync>>;
}

#[derive(Debug)]
pub struct ClientFrame<'frame> {
    client: &'frame mut Client,
    wait_policy: WaitPolicy,
}
impl<'frame> ClientFrame<'frame> {
    pub fn prefetch(&mut self) -> ClientFrame {
        ClientFrame {
            client: self.client,
            wait_policy: WaitPolicy::Prefetch,
        }
    }

    fn get_impl(&self, url: &str, parser: Box<dyn DynParser>) -> Response<()> {
        let (request, response) = {
            let cache = self.client.cache.cache.lock().unwrap();
            if let Some(entry) = cache.get(url) {
                if entry.parsed.is_some() {
                    (false, Some(Response::Ok(())))
                } else if let Some((_, response)) = &entry.last_refresh {
                    (false, Some(response.clone()))
                } else {
                    panic!("Unexpected cache state");
                }
            } else {
                (true, None)
            }
        };

        if !request {
            return response.unwrap();
        }

        let Some(helper) = &self.client.helper else {
            return response.unwrap_or(Response::Offline);
        };

        let mut state = helper.state.lock().unwrap();
        if !state.running {
            return response.unwrap_or(Response::Offline);
        }

        state.add_request(url.to_string(), parser);
        helper.helper_wakeup.notify_all();

        loop {
            if !state.running {
                return response.unwrap_or(Response::Offline);
            }

            match self.wait_policy {
                WaitPolicy::Wait => {
                    state = helper.response_notify.wait(state).unwrap();
                }
                WaitPolicy::Deadline(deadline) => {
                    let timed_out;
                    (state, timed_out) = helper.response_notify.wait_timeout(state, deadline - Instant::now()).unwrap();
                    if timed_out.timed_out() {
                        return response.unwrap_or(Response::Pending);
                    }
                }
                WaitPolicy::Prefetch => {
                    return response.unwrap_or(Response::Pending);
                }
            }

            if let Some(entry) = self.client.cache.cache.lock().unwrap().get(url) {
                if entry.parsed.is_some() {
                    return Response::Ok(());
                } else if let Some((_, response)) = &entry.last_refresh {
                    return response.clone();
                } else {
                    panic!("Unexpected cache state");
                }
            }
        }
    }

    fn get<'a, T: DeserializeOwned + Clone + Send + Sync + 'static>(&self, url: impl Into<Cow<'a, str>>) -> Response<T> {
        struct Parser<T>(std::marker::PhantomData<T>);
        impl<T> std::fmt::Debug for Parser<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Parser<{}>", std::any::type_name::<T>())
            }
        }
        impl<T: DeserializeOwned + Send + Sync + 'static> DynParser for Parser<T> {
            fn parse(&self, s: &str) -> Result<Box<dyn Any + Send + Sync>> {
                let data: T = serde_json::from_str(s)?;
                Ok(Box::new(data))
            }
        }
        let url: String = url.into().into();

        // NOTE: The type-erased get_impl can't return a reference to the parsed result
        //       because its lifetime ends when the cache lock is dropped.
        //       We re-lock and re-check, which is not ideal but works because we
        //       never remove cache entries.
        //
        //       It should be possible to fix that once MutexGuard::map becomes stable.
        self.get_impl(&url, Box::new(Parser::<T>(std::marker::PhantomData)))
            .map(|_| {
                self.client.cache.cache
                    .lock().unwrap()
                    .get(&url).unwrap()
                    .parsed.as_ref().unwrap()
                    .downcast_ref::<T>().unwrap()
                    .clone()
            })
    }

    pub fn pull<'a>(&self, organization: impl Into<Cow<'a, str>>, gh_repo: impl Into<Cow<'a, str>>, pull: u64) -> Response<api::Pull> {
        self.get(format!("repos/{}/{}/pulls/{}", organization.into(), gh_repo.into(), pull))
    }

    pub fn reviews<'a>(&self, organization: impl Into<Cow<'a, str>>, gh_repo: impl Into<Cow<'a, str>>, pull: u64) -> Response<Vec<api::Review>> {
        self.get(format!("repos/{}/{}/pulls/{}/reviews", organization.into(), gh_repo.into(), pull))
    }

    /// End the frame.
    ///
    /// The given function may be called from another thread (up to the start of the next frame)
    /// if one of the responses from this frame became stale.
    pub fn notify<F>(self, f: F)
    where
        F: FnOnce() + Send + Sync,
    {
        todo!()
    }
}
impl Drop for ClientFrame<'_> {
    fn drop(&mut self) {
        if let Some(helper) = &self.client.helper {
            let mut state = helper.state.lock().unwrap();
            let state = state.deref_mut();
            state.backlog_requests.append(&mut state.frame_requests);
        }
    }
}

#[derive(Debug, Clone)]
pub enum Response<T> {
    Ok(T),
    Pending,
    Offline,
    NotFound,
    Err(String),
}
impl<T> Response<T> {
    pub fn ok(self) -> std::result::Result<T, Cow<'static, str>> {
        match self {
            Response::Ok(value) => Ok(value),
            Response::Pending => Err(Cow::Borrowed(&"Waiting for response from server")),
            Response::Offline => Err(Cow::Borrowed(&"Not available (we're offline)")),
            Response::NotFound => Err(Cow::Borrowed(&"Not found")),
            Response::Err(err) => Err(Cow::Owned(err)),
        }
    }

    pub fn map<U, F>(self, f: F) -> Response<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Response::Ok(value) => Response::Ok(f(value)),
            Response::Pending => Response::Pending,
            Response::Offline => Response::Offline,
            Response::NotFound => Response::NotFound,
            Response::Err(err) => Response::Err(err),
        }
    }
}

#[derive(Debug, Default)]
struct CacheEntry {
    parsed: Option<Box<dyn Any + Send + Sync>>,
    last_refresh: Option<(Instant, Response<()>)>,
}

#[derive(Debug, Default)]
struct Cache {
    cache: Mutex<HashMap<String, CacheEntry>>,
}

#[derive(Debug)]
struct HelperCtrl {
    response_notify: Condvar,
    helper_wakeup: Condvar,
    state: Mutex<HelperState>,
}

#[derive(Debug)]
struct Request {
    url: String,
    parser: Box<dyn DynParser>,
}

#[derive(Debug)]
struct HelperState {
    running: bool,
    frame_requests: Vec<Request>,
    backlog_requests: Vec<Request>,
}
impl HelperState {
    fn add_request(&mut self, url: String, parser: Box<dyn DynParser>) {
        if let Some((idx, _)) = self.backlog_requests.iter().find_position(|r| r.url == url) {
            self.backlog_requests.remove(idx);
        } else if self.frame_requests.iter().any(|r| r.url == url) {
            return;
        }

        self.frame_requests.push(Request { url, parser });
    }
}

fn do_request(client: &reqwest::blocking::Client, url_api: &Url, url: &str, parser: Box<dyn DynParser>) -> Result<Response<Box<dyn Any + Send + Sync>>> {
    let url = url_api.join(url).unwrap();
    info!("Requesting {}", url);

    let response = client.get(url).send()?;
    debug!("Response: {:?}", &response);

    let converted =
        if response.status().is_success() {
            match parser.parse(&response.text()?) {
                Ok(parsed) => Response::Ok(parsed),
                Err(err) => Response::Err(format!("Error parsing response: {}", err)),
            }
        } else if response.status() == StatusCode::NOT_FOUND {
            Response::NotFound
        } else {
            Response::Err(format!("HTTP error: {}", response.status()))
        };

    Ok(converted)
}

fn run_helper(cache: Arc<Cache>, ctrl: Arc<HelperCtrl>, host: Host, url_api: Url) {
    let result = || -> Result<()> {
        let mut default_headers = header::HeaderMap::new();
        default_headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", host.token).parse()?,
        );
        default_headers.insert(header::ACCEPT, "application/vnd.github+json".parse()?);
        default_headers.insert("X-GitHub-Api-Version", "2022-11-28".parse()?);

        let client = reqwest::blocking::Client::builder()
            .user_agent("git-review")
            .default_headers(default_headers)
            .build()?;

        let mut state = ctrl.state.lock().unwrap();
        while state.running {
            let request =
                if !state.frame_requests.is_empty() {
                    state.frame_requests.drain(0..1).next()
                } else {
                    state.backlog_requests.pop()
                };
            let Some(request) = request else {
                state = ctrl.helper_wakeup.wait(state).unwrap();
                continue;
            };
            std::mem::drop(state);

            let response =
                match do_request(&client, &url_api, &request.url, request.parser) {
                    Ok(response) => response,
                    Err(err) => {
                        error!("Error processing request: {}", err);
                        Response::Err(err.to_string())
                    }
                };

            // Re-acquire the control lock *before* updating the cache.
            //
            // This ensures that response notifications aren't lost.
            state = ctrl.state.lock().unwrap();
            {
                let mut cache = cache.cache.lock().unwrap();
                let entry = cache.entry(request.url).or_default();

                let mut parsed = None;
                let response = response.map(|v| {
                    parsed = Some(v);
                    ()
                });

                if parsed.is_some() || matches!(response, Response::NotFound) {
                    entry.parsed = parsed;
                }
                entry.last_refresh = Some((Instant::now(), response));
            }

            ctrl.response_notify.notify_all();
        }

        Ok(())
    }();

    if let Err(err) = result {
        error!("Error in helper thread: {}", err);
    }
}
