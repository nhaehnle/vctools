use clap::Parser;
use diff_modulo_base::*;
use git_review::*;
use vctools_utils::prelude::*;

#[derive(Parser, Debug)]
struct Cli {
    file: std::path::PathBuf,
}

fn do_main() -> Result<()> {
    let args = Cli::parse();

    let bytes = utils::read_bytes(args.file)?;
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
