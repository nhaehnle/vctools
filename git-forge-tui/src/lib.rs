// SPDX-License-Identifier: GPL-3.0-or-later

mod config;
pub mod github;
pub mod logview;
pub mod tui;

pub use config::{get_project_dirs,load_config};
