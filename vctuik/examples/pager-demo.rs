// SPDX-License-Identifier: GPL-3.0-or-later

use clap::Parser;
use vctools_utils::prelude::*;
use vctuik::pager;

#[derive(Parser, Debug)]
struct Cli {
    file: std::path::PathBuf,
}

fn do_main() -> Result<()> {
    let args = Cli::parse();

    let bytes = vctools_utils::files::read_bytes(args.file)?;
    let text = String::from_utf8_lossy(&bytes);

    pager::run(text.into())?;

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
