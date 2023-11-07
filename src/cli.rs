// SPDX-License-Identifier: MIT

use std::io::IsTerminal;
use std::process::{Command, Stdio};

use clap::Args;
use termcolor::{ColorChoice, StandardStream, WriteColor};

#[derive(Debug, Clone, Default, Args)]
pub struct Options {
    /// Whether the output should be run through a pager
    #[clap(long)]
    pub pager: Option<bool>,

    /// Whether the output should be colored
    #[clap(long)]
    pub color: Option<bool>,
}

pub struct Cli {
    stream: Option<Box<dyn WriteColor>>,
    pager: Option<std::process::Child>,
}
impl Cli {
    pub fn new(options: Options) -> Cli {
        let is_terminal = std::io::stdout().is_terminal();

        // TODO: Take environment variables into account?
        let use_pager = options.pager.unwrap_or(is_terminal);
        let use_color = options.color.unwrap_or(is_terminal);

        let mut pager = use_pager.then(|| Command::new("less")
                .arg("-FR")
                .stdin(Stdio::piped())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn().ok()).flatten();

        let stream: Box<dyn WriteColor>;
        if let Some(pager) = &mut pager {
            if use_color {
                stream = Box::new(termcolor::Ansi::new(pager.stdin.take().unwrap()));
            } else {
                stream = Box::new(termcolor::NoColor::new(pager.stdin.take().unwrap()));
            }
        } else {
            let color = if use_color { ColorChoice::Always } else { ColorChoice::Never };
            stream = Box::new(StandardStream::stdout(color));
        }

        Cli {
            stream: Some(stream),
            pager,
        }
    }

    pub fn stream(&mut self) -> &mut dyn WriteColor {
        self.stream.as_mut().unwrap()
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        // Close the stream to signal EOF to the pager, if any.
        self.stream = None;

        // Wait for the pager to exit, otherwise it ends up killed by the shell
        // and leaves the terminal in a bad state.
        if let Some(pager) = &mut self.pager {
            // We don't *really* care if the wait failed -- it's best effort.
            pager.wait().unwrap_or_default();
        }
    }
}
