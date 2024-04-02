// SPDX-License-Identifier: MIT

use clap::{Parser, Subcommand, ValueEnum};

use diff_modulo_base::*;
use utils::*;

#[derive(ValueEnum, Debug, Clone, Copy)]
enum DiffAlgorithm {
    GraphSearch,
    SweepLine,
    SweepLineExact,
}
impl Default for DiffAlgorithm {
    fn default() -> Self {
        Self::SweepLine
    }
}
impl From<DiffAlgorithm> for diff::DiffAlgorithm {
    fn from(algorithm: DiffAlgorithm) -> Self {
        match algorithm {
            DiffAlgorithm::GraphSearch => Self::GraphSearch,
            DiffAlgorithm::SweepLine => Self::SweepLine,
            DiffAlgorithm::SweepLineExact => Self::SweepLineExact,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    Compose {
        first: std::path::PathBuf,
        second: std::path::PathBuf,
    },
    Diff {
        old: std::path::PathBuf,
        new: std::path::PathBuf,

        #[clap(value_enum, short, long, default_value_t = Default::default())]
        algorithm: DiffAlgorithm,
    },
}

#[derive(Parser, Debug)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

fn do_main() -> Result<()> {
    let args = Cli::parse();

    let mut buffer = diff::Buffer::new();

    match args.command {
        Command::Compose { first, second } => {
            let first_diff = utils::read_diff(&mut buffer, &first)?;
            let second_diff = utils::read_diff(&mut buffer, &second)?;

            let result_diff = diff::compose(&first_diff, &second_diff)?;
            print!("{}", result_diff.display_lossy(&buffer));
        }
        Command::Diff {
            old,
            new,
            algorithm,
        } => {
            let old_body = buffer.insert(&utils::read_bytes(&old)?)?;
            let new_body = buffer.insert(&utils::read_bytes(&new)?)?;

            let old_path = buffer.insert(old.to_string_lossy().as_bytes())?;
            let new_path = buffer.insert(new.to_string_lossy().as_bytes())?;

            let options = diff::DiffOptions {
                strip_path_components: 0,
                ..Default::default()
            };
            let file = try_forward(
                || {
                    diff::diff_file(
                        &buffer,
                        old_path,
                        new_path,
                        old_body,
                        new_body,
                        &options,
                        algorithm.into(),
                    )
                },
                || "diffing",
            )?;

            let mut diff = diff::Diff::new(options);
            diff.add_file(file);
            print!("{}", diff.display_lossy(&buffer));
        }
    }

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
