// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    any::Any,
    borrow::Cow,
    collections::{hash_map, HashMap},
    ops::DerefMut,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};

use itertools::Itertools;
use log::{debug, error, info, warn};
use reqwest::{header, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize};
use vctools_utils::{files, prelude::*};
use vctuik::signals::MergeWakeupSignal;

pub mod api;
pub mod connections;
pub mod edit;

use edit::Edit;

#[derive(Deserialize, Debug, Clone)]
pub struct Host {
    pub host: String,
    pub api: String,
    pub user: String,
    pub token: String,
    #[serde(default)]
    pub alias: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    host: Host,
    offline: bool,
    cache_dir: Option<PathBuf>,
}
impl ClientConfig {
    pub fn offline(self, offline: bool) -> Self {
        Self { offline, ..self }
    }

    pub fn cache_dir(self, cache_dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(cache_dir),
            ..self
        }
    }

    pub fn maybe_cache_dir(self, cache_dir: Option<PathBuf>) -> Self {
        Self { cache_dir, ..self }
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
        self.cache_dir
            .as_ref()
            .map(|dir| dir.join(url.replace('/', "%")))
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
                edit_requests: Vec::new(),
                response_signal: ResponseSignal::Disabled,
                response_callback: None,
            }),
        });
        self.helper = Some(helper.clone());

        let cache = self.cache.clone();
        let config = self.config.clone();
        let url_api = self.url_api.clone();

        std::thread::spawn(move || {
            run_helper(cache, helper, config, url_api);
        });

        Ok(())
    }

    pub fn start_frame(&mut self, deadline: Option<Instant>) {
        assert!(self.frame.is_none());

        self.frame = Some(if let Some(deadline) = deadline {
            WaitPolicy::Deadline(deadline)
        } else {
            WaitPolicy::Wait
        });

        if let Some(helper) = &self.helper {
            let mut state = helper.state.lock().unwrap();
            let state = state.deref_mut();
            state.backlog_requests.append(&mut state.frame_requests);
            state.response_signal = ResponseSignal::Disabled;
            state.response_callback = None;
        }
    }

    pub fn access(&mut self) -> ClientRef<'_> {
        let wait_policy = self.frame.unwrap();
        ClientRef {
            client: self,
            wait_policy,
        }
    }

    pub fn prefetch(&mut self) -> ClientRef<'_> {
        assert!(self.frame.is_some());
        ClientRef {
            client: self,
            wait_policy: WaitPolicy::Prefetch,
        }
    }

    pub fn edit(&mut self, edit: Edit) -> Result<()> {
        assert!(self.frame.is_some());

        let Some(helper) = &self.helper else {
            return Err("Cannot perform edits while offline")?;
        };
        let mut state = helper.state.lock().unwrap();

        {
            struct ItemGetter<'a> {
                cache: &'a mut HashMap<String, CacheEntry>,
            }
            impl<'a> edit::ItemGetter for ItemGetter<'a> {
                fn get(&mut self, url: &str) -> Option<&mut Box<dyn Any + Send + Sync>> {
                    self.cache.get_mut(url).and_then(|entry| entry.parsed.as_mut())
                }
            }

            let mut cache = self.cache.cache.lock().unwrap();
            edit.apply(&mut ItemGetter { cache: cache.deref_mut() });
        }

        state.edit_requests.push(edit);

        helper.helper_wakeup.notify_all();

        Ok(())
    }

    pub fn end_frame(&mut self, notify: Option<&MergeWakeupSignal>) {
        assert!(self.frame.is_some());

        self.frame = None;

        if let Some((helper, notify)) = self.helper.as_ref().zip(notify) {
            let mut state = helper.state.lock().unwrap();
            if state.response_signal == ResponseSignal::Pending {
                debug!("Signaling callback from end of frame");
                notify.signal();
                state.response_signal = ResponseSignal::Signaled;
            } else if state.response_signal == ResponseSignal::Requested {
                state.response_callback = Some(notify.clone());
            }
        }
    }
}

trait DynParser: std::fmt::Debug + Send + Sync {
    fn parse(&self, s: &str) -> Result<Box<dyn Any + Send + Sync>>;
}

fn load_from_cache(
    cache_file: &Path,
    parser: &dyn DynParser,
) -> Response<Box<dyn Any + Send + Sync>> {
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
        Err(err) => Response::Err(format!(
            "Error reading cache file {}: {}",
            cache_file.display(),
            err
        )),
    }
}

#[derive(Debug)]
pub struct ClientRef<'frame> {
    client: &'frame mut Client,
    wait_policy: WaitPolicy,
}
impl<'frame> ClientRef<'frame> {
    fn get_impl(&self, url: &str, parser: Box<dyn DynParser>) -> Response<()> {
        let (request_now, request_pending, response) = {
            let mut cache = self.client.cache.cache.lock().unwrap();
            match cache.entry(url.into()) {
                hash_map::Entry::Occupied(entry) => {
                    let entry = entry.get();
                    (false, entry.fetched.is_none(), entry.response.clone())
                }
                hash_map::Entry::Vacant(entry) => {
                    let mut response = Response::Pending;
                    if let Some(cache_file) = self.client.config.cache_for_url(url) {
                        response = load_from_cache(cache_file.as_ref(), parser.as_ref())
                    }

                    let (parsed, response) = response.split();

                    entry.insert(CacheEntry {
                        fetched: None,
                        response: response.clone(),
                        parsed,
                    });

                    (true, false, response)
                }
            }
        };

        let Some(helper) = &self.client.helper else {
            return response.pending_to_offline();
        };
        let mut state = helper.state.lock().unwrap();
        if !state.running {
            return response.pending_to_offline();
        }

        let is_prefetch = matches!(self.wait_policy, WaitPolicy::Prefetch);
        if request_now {
            state.add_request(url.to_string(), parser, is_prefetch);
            helper.helper_wakeup.notify_all();
        }

        if !request_now && !request_pending {
            return response;
        }

        if is_prefetch {
            if state.response_signal == ResponseSignal::Disabled {
                state.response_signal = ResponseSignal::Requested;
            }
            return response;
        }

        loop {
            if !state.running {
                return response.pending_to_offline();
            }

            match self.wait_policy {
                WaitPolicy::Wait => {
                    state = helper.response_notify.wait(state).unwrap();
                }
                WaitPolicy::Deadline(deadline) => {
                    let timed_out;
                    (state, timed_out) = helper
                        .response_notify
                        .wait_timeout(state, deadline - Instant::now())
                        .unwrap();
                    if timed_out.timed_out() {
                        if state.response_signal == ResponseSignal::Disabled {
                            state.response_signal = ResponseSignal::Requested;
                        }
                        return response;
                    }
                }
                WaitPolicy::Prefetch => unreachable!(),
            }

            if let Some(entry) = self.client.cache.cache.lock().unwrap().get(url) {
                if entry.fetched.is_some() {
                    return entry.response.clone();
                }
            }
        }
    }

    fn get<'a, T: DeserializeOwned + Clone + Send + Sync + 'static>(
        &self,
        url: impl Into<Cow<'a, str>>,
    ) -> Response<T> {
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
                self.client
                    .cache
                    .cache
                    .lock()
                    .unwrap()
                    .get(&url)
                    .unwrap()
                    .parsed
                    .as_ref()
                    .unwrap()
                    .downcast_ref::<T>()
                    .unwrap()
                    .clone()
            })
    }

    pub fn pull<'a>(
        &self,
        organization: impl Into<Cow<'a, str>>,
        gh_repo: impl Into<Cow<'a, str>>,
        pull: u64,
    ) -> Response<api::Pull> {
        self.get(format!(
            "repos/{}/{}/pulls/{}",
            organization.into(),
            gh_repo.into(),
            pull
        ))
    }

    pub fn reviews<'a>(
        &self,
        organization: impl Into<Cow<'a, str>>,
        gh_repo: impl Into<Cow<'a, str>>,
        pull: u64,
    ) -> Response<Vec<api::Review>> {
        self.get(format!(
            "repos/{}/{}/pulls/{}/reviews",
            organization.into(),
            gh_repo.into(),
            pull
        ))
    }

    /// Returns the comments on an issue (including non-review comments on a PR).
    pub fn issue_comments<'a>(
        &self,
        organization: impl Into<Cow<'a, str>>,
        gh_repo: impl Into<Cow<'a, str>>,
        number: u64,
    ) -> Response<Vec<api::Comment>> {
        self.get(format!(
            "repos/{}/{}/issues/{}/comments",
            organization.into(),
            gh_repo.into(),
            number,
        ))
    }

    /// Returns unread notifications (like github.com/notifications).
    ///
    /// The API seems to be unable to report the "done" state of notification
    /// threads, so we only ever show unread notifications, and markting them
    /// "done" marks them both read and done.
    pub fn notifications<'a>(&self) -> Response<Vec<api::NotificationThread>> {
        self.get("notifications")
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

    pub fn ok_or_pending(self) -> std::result::Result<Option<T>, Cow<'static, str>> {
        match self {
            Response::Ok(value) => Ok(Some(value)),
            Response::Pending => Ok(None),
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

    pub fn pending_to_offline(self) -> Response<T> {
        match self {
            Response::Pending => Response::Offline,
            other => other,
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseSignal {
    /// No signal required when a response is received.
    Disabled,

    /// Should signal when a response is received.
    Requested,

    /// Response has been received and should be signaled, but no callback has
    /// been registered yet.
    Pending,

    /// Response has been received and signaled.
    Signaled,
}

struct HelperState {
    running: bool,
    frame_requests: Vec<Request>,
    backlog_requests: Vec<Request>,
    edit_requests: Vec<Edit>,
    response_signal: ResponseSignal,
    response_callback: Option<MergeWakeupSignal>,
}
impl std::fmt::Debug for HelperState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperState")
            .field("running", &self.running)
            .field("frame_requests", &self.frame_requests.len())
            .field("backlog_requests", &self.backlog_requests.len())
            .field("edit_requests", &self.edit_requests.len())
            .field("response_signal", &self.response_signal)
            .field(
                "response_callback",
                if self.response_callback.is_some() {
                    &"Some(...)"
                } else {
                    &"None"
                },
            )
            .finish()
    }
}
impl HelperState {
    fn add_request(&mut self, url: String, parser: Box<dyn DynParser>, prefetch: bool) {
        if let Some((idx, _)) = self.backlog_requests.iter().find_position(|r| r.url == url) {
            if prefetch {
                return;
            }

            self.backlog_requests.remove(idx);
        } else if self.frame_requests.iter().any(|r| r.url == url) {
            return;
        }

        if prefetch {
            self.backlog_requests.push(Request { url, parser });
        } else {
            self.frame_requests.push(Request { url, parser });
        }
    }
}

fn do_request(
    client: &reqwest::blocking::Client,
    url_api: &Url,
    url: &str,
    cache_file: Option<PathBuf>,
    parser: Box<dyn DynParser>,
) -> Result<Response<Box<dyn Any + Send + Sync>>> {
    let url = url_api.join(url).unwrap();
    info!("Requesting {}", url);

    let response = client.get(url).send()?;
    debug!("Response: {:?}", &response);

    let converted = if response.status().is_success() {
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
            // Commit edits first.
            if !state.edit_requests.is_empty() {
                let edit = state.edit_requests.drain(0..1).next().unwrap();
                std::mem::drop(state);

                info!("Committing edit {:?}", edit);

                if let Err(err) = edit.commit(&client, &url_api) {
                    error!("Error committing edit {:?}: {}", edit, err);
                }

                state = ctrl.state.lock().unwrap();
                continue;
            }

            // Now handle requests.
            let (request, is_backlog) = if !state.frame_requests.is_empty() {
                (state.frame_requests.drain(0..1).next(), false)
            } else {
                (state.backlog_requests.pop(), true)
            };
            let Some(request) = request else {
                state = ctrl.helper_wakeup.wait(state).unwrap();
                continue;
            };
            std::mem::drop(state);

            let response = match do_request(
                &client,
                &url_api,
                &request.url,
                config.cache_for_url(&request.url),
                request.parser,
            ) {
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
                let entry = cache.entry(request.url.clone()).or_default();

                let (mut parsed, response) = response.split();

                struct ItemGetter<'a> {
                    url: &'a str,
                    parsed: Option<&'a mut Box<dyn Any + Send + Sync>>,
                }
                impl<'a> edit::ItemGetter for ItemGetter<'a> {
                    fn get(&mut self, url: &str) -> Option<&mut Box<dyn Any + Send + Sync>> {
                        if self.url == url {
                            self.parsed.take()
                        } else {
                            None
                        }
                    }
                }

                for edit in &state.edit_requests {
                    edit.apply(&mut ItemGetter {
                        url: &request.url,
                        parsed: parsed.as_mut(),
                    });
                }

                if parsed.is_some() || matches!(response, Response::NotFound) {
                    entry.parsed = parsed;
                }

                entry.fetched = Some(Instant::now());
                entry.response = response;
            }

            if !is_backlog && state.response_signal == ResponseSignal::Requested {
                if let Some(callback) = &state.response_callback {
                    debug!("Signaling callback from response");
                    callback.signal();
                    state.response_signal = ResponseSignal::Signaled;
                } else {
                    state.response_signal = ResponseSignal::Pending;
                }
            }

            ctrl.response_notify.notify_all();
        }

        Ok(())
    }();

    if let Err(err) = result {
        error!("Error in helper thread: {}", err);
    }
}
