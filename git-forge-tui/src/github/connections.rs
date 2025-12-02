// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, collections::HashMap, path::PathBuf, time::Instant};

use serde::Deserialize;
use vctools_utils::prelude::*;
use vctuik::signals::MergeWakeupSignal;

use crate::github;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub hosts: Vec<github::Host>,
}

#[derive(Debug)]
struct LiveConfig {
    hosts: Vec<github::Host>,
    offline: bool,
    cache_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct Clients {
    have_all_clients: bool,
    clients: HashMap<String, Result<RefCell<github::Client>>>,
}
impl Clients {
    fn new() -> Self {
        Self {
            have_all_clients: false,
            clients: HashMap::new(),
        }
    }

    pub fn client(
        &mut self,
        config: &LiveConfig,
        deadline: Option<Instant>,
        hostname: String,
    ) -> Result<&RefCell<github::Client>> {
        self.clients
            .entry(hostname)
            .or_insert_with_key(|hostname: &String| -> Result<RefCell<github::Client>> {
                let Some(host) = config.hosts.iter().find(|h| h.matches_host(hostname)) else {
                    Err(format!(
                        "Host not configured; add it to your github.toml: {hostname}"
                    ))?
                };

                github::Client::build(host.clone())
                    .offline(config.offline)
                    .maybe_cache_dir(
                        config
                            .cache_dir
                            .as_ref()
                            .map(|cache_dir| cache_dir.join(&host.host)),
                    )
                    .new()
                    .map(|mut client| {
                        client.start_frame(deadline);
                        RefCell::new(client)
                    })
            })
            .as_ref_ok()
    }

    pub fn all_clients<'a>(
        &'a mut self,
        config: &'a LiveConfig,
        deadline: Option<Instant>,
    ) -> impl Iterator<Item = (&'a github::Host, Result<&'a RefCell<github::Client>>)> {
        if !self.have_all_clients {
            for host in &config.hosts {
                let _ = self.client(config, deadline, host.host.clone());
            }
            self.have_all_clients = true;
        }

        config.hosts.iter().map(|host| {
            let client = self.clients.get(&host.host).unwrap();
            (host, client.as_ref_ok())
        })
    }
}

#[derive(Debug)]
pub struct Connections {
    config: LiveConfig,
    clients: Clients,
    frame: Option<Option<Instant>>,
}
impl Connections {
    pub fn new(config: Config, offline: bool, cache_dir: Option<PathBuf>) -> Self {
        Self {
            config: LiveConfig {
                hosts: config.hosts,
                offline,
                cache_dir,
            },
            clients: Clients::new(),
            frame: None,
        }
    }

    pub fn hosts(&self) -> &[github::Host] {
        &self.config.hosts
    }

    pub fn start_frame(&mut self, deadline: Option<Instant>) {
        assert!(self.frame.is_none());
        self.frame = Some(deadline);

        for (_, client) in &mut self.clients.clients {
            if let Some(client) = client.as_mut().ok() {
                client.borrow_mut().start_frame(deadline);
            }
        }
    }

    pub fn end_frame(&mut self, notify: Option<&MergeWakeupSignal>) {
        assert!(self.frame.is_some());
        self.frame = None;

        for (_, client) in &mut self.clients.clients {
            if let Some(client) = client.as_mut().ok() {
                client.borrow_mut().end_frame(notify);
            }
        }
    }

    pub fn client(&mut self, host: impl Into<String>) -> Result<&RefCell<github::Client>> {
        // Only allowed between start_frame and end_frame
        let deadline = self.frame.unwrap();

        self.clients.client(&self.config, deadline, host.into())
    }

    pub fn all_clients(
        &mut self,
    ) -> impl Iterator<Item = (&github::Host, Result<&RefCell<github::Client>>)> {
        // Only allowed between start_frame and end_frame
        let deadline = self.frame.unwrap();
        self.clients.all_clients(&self.config, deadline)
    }
}
