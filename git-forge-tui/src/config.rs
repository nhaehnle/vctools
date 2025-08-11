// SPDX-License-Identifier: GPL-3.0-or-later

use directories::ProjectDirs;
use lazy_static::lazy_static;
use serde::de::DeserializeOwned;

use vctools_utils::{files, prelude::*};

pub fn get_project_dirs() -> &'static ProjectDirs {
    lazy_static! {
        static ref PROJECT_DIRS: ProjectDirs =
            ProjectDirs::from("experimental", "nhaehnle", "vctools").unwrap();
    }
    &PROJECT_DIRS
}

pub fn load_config_impl<C: DeserializeOwned>(name: &str) -> Result<C> {
    let dirs = ProjectDirs::from("experimental", "nhaehnle", "vctools").unwrap();
    let config: C = {
        let mut config = dirs.config_dir().to_path_buf();
        config.push(name);
        try_forward(
            || {
                Ok(toml::from_str(str::from_utf8(&files::read_bytes(
                    config,
                )?)?)?)
            },
            || format!("Error loading {name}"),
        )?
    };
    Ok(config)
}

pub fn load_config<'a, C: DeserializeOwned>(name: impl Into<&'a str>) -> Result<C> {
    load_config_impl(name.into())
}
