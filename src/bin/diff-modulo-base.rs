// SPDX-License-Identifier: MIT

use clap::Parser;

use diff_modulo_base::*;
use utils::Result;
use diff::ChunkFreeWriterExt;

#[derive(Parser, Debug)]
struct Cli {
    base_old_diff: std::path::PathBuf,
    base_new_diff: std::path::PathBuf,
    target_diff: std::path::PathBuf,
}

fn do_main() -> Result<()> {
    let args = Cli::parse();

    let mut buffer = diff::Buffer::new();
    let base_old_diff = utils::read_diff(&mut buffer, &args.base_old_diff)?;
    let base_new_diff = utils::read_diff(&mut buffer, &args.base_new_diff)?;
    let target_diff = utils::read_diff(&mut buffer, &args.target_diff)?;

    // println!("{:?}", &target_diff);

    // let result_diff = reduce_modulo_base(target_diff, &old_base_diff, &base_new_diff)?;
    let mut writer = diff::ChunkByteBufferWriter::new();
    diff::diff_modulo_base(&buffer, target_diff, &base_old_diff, &base_new_diff, &mut writer.with_buffer(&buffer))?;
    print!("{}", String::from_utf8_lossy(&writer.out));

    Ok(())
}

fn main() {
    if let Err(err) = do_main() {
        println!("{}", err);
        std::process::exit(1);
    }
}
