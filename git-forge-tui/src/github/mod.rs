// SPDX-License-Identifier: GPL-3.0-or-later

use std::{any::Any, borrow::Cow, collections::HashMap, ops::DerefMut, path::{Path, PathBuf}, sync::{Arc, Condvar, Mutex}, time::{Duration, Instant}};

use itertools::Itertools;
use log::{trace, debug, info, warn, error};
use reqwest::{header, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize};
use vctools_utils::{files, prelude::*};

pub mod api;
pub mod connections;

#[derive(Deserialize, Debug, Clone)]
pub struct Host {
    pub host: String,
    pub api: String,
    pub user: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    host: Host,
    offline: bool,
    cache_dir: Option<PathBuf>,
}
impl ClientConfig {
    pub fn offline(self, offline: bool) -> Self {
        Self {
            offline,
            ..self
        }
    }

    pub fn cache_dir(self, cache_dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(cache_dir),
            ..self
        }
    }

    pub fn maybe_cache_dir(self, cache_dir: Option<PathBuf>) -> Self {
        Self {
            cache_dir,
            ..self
        }
    }

    pub fn new(self) -> Result<Client> {
        let url_api = Url::parse(&self.host.api)?;

        if let Some(cache_dir) = &self.cache_dir {
            std::fs::create_dir_all(cache_dir)?;
        }

        let mut client = Client {
            config: self,
            url_api,
            cache: Arc::new(Cache::default()),
            helper: None,
            frame: None,
        };

        if !client.config.offline {
            client.start_thread()?;
        }

        Ok(client)
    }

    fn cache_for_url(&self, url: &str) -> Option<PathBuf> {
        self.cache_dir.as_ref().map(|dir| {
            dir.join(url.replace('/', "%"))
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum WaitPolicy {
    Wait,
    Deadline(Instant),
    Prefetch,
}

#[derive(Debug)]
pub struct Client {
    config: ClientConfig,
    url_api: Url,
    cache: Arc<Cache>,
    helper: Option<Arc<HelperCtrl>>,
    frame: Option<WaitPolicy>,
}
impl Client {
    pub fn build(host: Host) -> ClientConfig {
        ClientConfig {
            host,
            offline: false,
            cache_dir: None,
        }
    }

    pub fn host(&self) -> &Host {
        &self.config.host
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
        let config = self.config.clone();
        let url_api = self.url_api.clone();

        std::thread::spawn(move || {
            run_helper(
                cache,
                helper,
                config,
                url_api,
            );
        });

        Ok(())
    }

    pub fn start_frame(&mut self, deadline: Option<Instant>) {
        assert!(self.frame.is_none());

        self.frame = Some(
            if let Some(deadline) = deadline {
                WaitPolicy::Deadline(deadline)
            } else {
                WaitPolicy::Wait
            }
        );
    }

    pub fn access(&mut self) -> ClientRef {
        let wait_policy = self.frame.unwrap();
        ClientRef {
            client: self,
            wait_policy,
        }
    }

    pub fn prefetch(&mut self) -> ClientRef {
        assert!(self.frame.is_some());
        ClientRef {
            client: self,
            wait_policy: WaitPolicy::Prefetch,
        }
    }

    pub fn end_frame(&mut self) {
        assert!(self.frame.is_some());

        if let Some(helper) = &self.helper {
            let mut state = helper.state.lock().unwrap();
            let state = state.deref_mut();
            state.backlog_requests.append(&mut state.frame_requests);
        }

        self.frame = None;
    }
}

trait DynParser: std::fmt::Debug + Send + Sync {
    fn parse(&self, s: &str) -> Result<Box<dyn Any + Send + Sync>>;
}

fn load_from_cache(cache_file: &Path, parser: &dyn DynParser) -> Response<Box<dyn Any + Send + Sync>> {
    if !cache_file.exists() {
        return Response::Pending;
    }

    let result = || -> Result<_> {
        let bytes = files::read_bytes(cache_file)?;
        let string = str::from_utf8(&bytes)?;
        Ok(Response::Ok(parser.parse(string)?))
    }();

    match result {
    Ok(response) => response,
    Err(err) => Response::Err(format!("Error reading cache file {}: {}", cache_file.display(), err)),
    }
}

#[derive(Debug)]
pub struct ClientRef<'frame> {
    client: &'frame mut Client,
    wait_policy: WaitPolicy,
}
impl<'frame> ClientRef<'frame> {
    fn get_impl(&self, url: &str, parser: Box<dyn DynParser>) -> Response<()> {
        let (request, response) = {
            let mut cache = self.client.cache.cache.lock().unwrap();
            let entry =
                if let Some(entry) = cache.get(url) {
                    entry
                } else {
                    let response =
                        if let Some(cache_file) = self.client.config.cache_for_url(url) {
                            load_from_cache(cache_file.as_ref(), parser.as_ref())
                        } else {
                            Response::Pending
                        };

                        let (parsed, response) = response.split();
                    cache.entry(url.to_string())
                        .or_insert(CacheEntry {
                            fetched: None,
                            response: response.clone(),
                            parsed,
                        })
                };

            let response =
                if matches!(entry.response, Response::Pending) {
                    None
                } else {
                    Some(entry.response.clone())
                };
            (entry.fetched.is_none(), response)
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
                if entry.fetched.is_some() {
                    if entry.parsed.is_some() {
                        return Response::Ok(());
                    } else {
                        return entry.response.clone();
                    }
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

    pub fn split(self) -> (Option<T>, Response<()>) {
        let mut data = None;
        let response = self.map(|v| {
            data = Some(v);
            ()
        });
        (data, response)
    }
}

#[derive(Debug)]
struct CacheEntry {
    response: Response<()>,
    fetched: Option<Instant>,
    parsed: Option<Box<dyn Any + Send + Sync>>,
}
impl Default for CacheEntry {
    fn default() -> Self {
        Self {
            response: Response::Pending,
            fetched: None,
            parsed: None,
        }
    }
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

fn do_request(
    client: &reqwest::blocking::Client,
    url_api: &Url,
    url: &str,
    cache_file: Option<PathBuf>,
    parser: Box<dyn DynParser>)
-> Result<Response<Box<dyn Any + Send + Sync>>>
{
    let url = url_api.join(url).unwrap();
    info!("Requesting {}", url);

    let response = client.get(url).send()?;
    debug!("Response: {:?}", &response);

    let converted =
        if response.status().is_success() {
            let text = response.text()?;

            if let Some(cache_file) = cache_file {
                if let Err(err) = std::fs::write(&cache_file, text.as_bytes()) {
                    warn!("Error writing cache file {}: {}", cache_file.display(), err);
                }
            }

            match parser.parse(&text) {
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

fn run_helper(cache: Arc<Cache>, ctrl: Arc<HelperCtrl>, config: ClientConfig, url_api: Url) {
    let result = || -> Result<()> {
        let mut default_headers = header::HeaderMap::new();
        default_headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", config.host.token).parse()?,
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
                match do_request(&client, &url_api, &request.url, config.cache_for_url(&request.url), request.parser) {
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

                let (parsed, response) = response.split();

                if parsed.is_some() || matches!(response, Response::NotFound) {
                    entry.parsed = parsed;
                }

                entry.fetched = Some(Instant::now());
                entry.response = response;
            }

            ctrl.response_notify.notify_all();
        }

        Ok(())
    }();

    if let Err(err) = result {
        error!("Error in helper thread: {}", err);
    }
}
