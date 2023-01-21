// SPDX-License-Identifier: MIT

use diff_modulo_base::*;
use utils::Result;

#[test]
fn dmb_test() -> Result<()> {
    for entry in std::path::Path::new("./tests/dmb_test").read_dir()? {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name == "." || file_name == ".." {
            continue
        }
        if !entry.file_type()?.is_dir() {
            continue
        }

        let path = entry.path();
        println!("Test: {}", path.display());

        let mut buffer = diff::Buffer::new();
        let old_base_diff = utils::read_diff(&mut buffer, path.join("old.diff"))?;
        let new_base_diff = utils::read_diff(&mut buffer, path.join("new.diff"))?;
        let target_diff = utils::read_diff(&mut buffer, path.join("target.diff"))?;
        let expected = utils::read_bytes(path.join("expected.diff"))?;

        let result = diff::diff_modulo_base(&buffer, target_diff, &old_base_diff, &new_base_diff)?;

        assert_eq!(expected, result);
    }

    Ok(())
}
