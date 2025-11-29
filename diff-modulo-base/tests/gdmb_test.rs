// SPDX-License-Identifier: MIT

use std::os::unix::ffi::OsStringExt;

use clap::Parser;
use diff_modulo_base::*;
use utils::Result;

#[test]
fn dmb_test() -> Result<()> {
    for entry in std::path::Path::new("./tests/gdmb_test").read_dir()? {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name == "." || file_name == ".." {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let path = entry.path();
        println!("Test: {}", path.display());

        let cmdline = utils::read_bytes(path.join("test-cmdline"))?;
        let expected = utils::read_bytes(path.join("test-expected"))?;

        let args: Vec<_> = [std::ffi::OsString::from("git-diff-modulo-base")]
            .into_iter()
            .chain(
                cmdline
                    .split(|&ch| ch == b' ' || ch == b'\n')
                    .filter_map(|slice| {
                        if slice.is_empty() {
                            None
                        } else {
                            Some(std::ffi::OsString::from_vec(slice.to_vec()))
                        }
                    }),
            )
            .collect();
        println!("{:?}", &args);
        let args = tool::GitDiffModuloBaseArgs::try_parse_from(args)?;

        let repo = git_core::Repository::new(std::path::PathBuf::from("."));
        let mut ep = git_core::MockExecutionProvider {
            mock_data_path: path,
        };

        let mut out_buffer: Vec<u8> = Vec::new();
        let mut out = termcolor::NoColor::new(&mut out_buffer);

        let mut writer = diff_color::Writer::new();
        tool::git_diff_modulo_base(&args, &repo, &mut ep, &mut writer)?;
        writer.write(&mut out)?;

        assert_eq!(
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&out_buffer)
        );
    }

    Ok(())
}
