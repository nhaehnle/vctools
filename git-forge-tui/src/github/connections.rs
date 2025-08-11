// SPDX-License-Identifier: GPL-3.0-or-later

use std::{collections::HashMap, path::PathBuf, time::Instant};

use serde::Deserialize;
use vctools_utils::prelude::*;

use crate::github;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub hosts: Vec<github::Host>,
}

#[derive(Debug)]
pub struct Connections {
    config: Config,
    offline: bool,
    cache_dir: Option<PathBuf>,
    clients: HashMap<String, Result<github::Client>>,
    frame: Option<Option<Instant>>,
}
impl Connections {
    pub fn new(config: Config, offline: bool, cache_dir: Option<PathBuf>) -> Self {
        Self {
            config,
            offline,
            cache_dir,
            clients: HashMap::new(),
            frame: None,
        }
    }

    pub fn start_frame(&mut self, deadline: Option<Instant>) {
        assert!(self.frame.is_none());
        self.frame = Some(deadline);

        for (_, client) in &mut self.clients {
            if let Some(client) = client.as_mut().ok() {
                client.start_frame(deadline);
            }
        }
    }

    pub fn end_frame(&mut self) {
        assert!(self.frame.is_some());
        self.frame = None;

        for (_, client) in &mut self.clients {
            if let Some(client) = client.as_mut().ok() {
                client.end_frame();
            }
        }
    }

    fn client_impl(&mut self, host: String) -> Result<&mut github::Client> {
        // Only allowed between start_frame and end_frame
        let deadline = self.frame.as_ref().unwrap();

        self.clients.entry(host)
            .or_insert_with_key(|host| -> Result<github::Client> {
                let Some(config) =
                    self.config.hosts.iter()
                        .find(|h| h.host == *host)
                else {
                    Err(format!("Host not configured; add it to your github.toml: {host}"))?
                };

                github::Client::build(config.clone())
                    .offline(self.offline)
                    .maybe_cache_dir(self.cache_dir.clone())
                    .new()
                    .map(|mut client| {
                        client.start_frame(*deadline);
                        client
                    })
                    .into()
            })
            .as_mut_ok()
    }

    pub fn client(&mut self, host: impl Into<String>) -> Result<&mut github::Client> {
        self.client_impl(host.into())
    }
}
