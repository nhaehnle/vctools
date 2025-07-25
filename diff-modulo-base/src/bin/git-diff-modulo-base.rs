// SPDX-License-Identifier: MIT

use diff_modulo_base::*;
use utils::Result;

use clap::Parser;

#[derive(Parser, Debug)]
pub struct Options {
    #[clap(flatten)]
    pub gdmb: tool::GitDiffModuloBaseArgs,

    /// Behave as if run from the given path.
    #[clap(short = 'C', default_value = ".")]
    pub path: std::path::PathBuf,

    #[clap(flatten)]
    pub cli: cli::Options,
}

fn do_main() -> Result<()> {
    let args = Options::parse();
    let mut cli = cli::Cli::new(args.cli);
    let out = cli.stream();

    let repo = git_core::Repository::new(&args.path);

    let mut writer = diff_color::Writer::new();
    tool::git_diff_modulo_base(args.gdmb, repo, &mut writer)?;
    writer.write(out)?;

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
