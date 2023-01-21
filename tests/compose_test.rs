// SPDX-License-Identifier: MIT

use diff_modulo_base::*;
use utils::Result;

#[test]
fn compose_test() -> Result<()> {
    for entry in std::path::Path::new("./tests/compose_test").read_dir()? {
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
        let first_diff = utils::read_diff(&mut buffer, path.join("first.diff"))?;
        let second_diff = utils::read_diff(&mut buffer, path.join("second.diff"))?;
        let expected = utils::read_bytes(path.join("expected.diff"))?;

        let result_diff = diff::compose(&first_diff, &second_diff)?;
        let result = result_diff.render(&buffer);

        assert_eq!(expected, result);
    }

    Ok(())
}
