// SPDX-License-Identifier: GPL-3.0-or-later

use std::any::Any;

use log::{debug, info};
use reqwest::Url;
use vctools_utils::prelude::*;

use super::api;

pub trait ItemGetter {
    fn get(&mut self, url: &str) -> Option<&mut Box<dyn Any + Send + Sync>>;
}

#[derive(Debug, Clone)]
pub enum Edit {
    MarkNotificationDone(String),
}
impl Edit {
    pub fn apply(&self, getter: &mut dyn ItemGetter) {
        match self {
            Edit::MarkNotificationDone(id) => {
                if let Some(item) = getter.get("notifications") {
                    if let Some(threads) = item.downcast_mut::<Vec<api::NotificationThread>>() {
                        threads.retain(|thread| thread.id != *id);
                    }
                }
            },
        }
    }

    pub fn commit(&self, client: &reqwest::blocking::Client, url_api: &Url) -> Result<()> {
        match self {
            Edit::MarkNotificationDone(id) => {
                let url = url_api.join(&format!("notifications/threads/{id}")).unwrap();
                info!("Deleting {}", url);

                let response = client.delete(url).send()?;
                debug!("Response: {:?}", &response);

                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(format!(
                        "Failed to mark notification thread {id} as done: HTTP {}",
                        response.status()
                    ))?
                }
            }
        }
    }
}
