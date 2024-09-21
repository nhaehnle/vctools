// SPDX-License-Identifier: MIT

use diff::ChunkFreeWriterExt;
use diff_modulo_base::*;
use utils::Result;

#[test]
fn diff_test() -> Result<()> {
    for entry in std::path::Path::new("./tests/diff_test").read_dir()? {
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

        let mut buffer = diff::Buffer::new();
        let old_path = path.join("old");
        let new_path = path.join("new");
        let old_body = buffer.insert(&utils::read_bytes(&old_path)?)?;
        let new_body = buffer.insert(&utils::read_bytes(&new_path)?)?;
        let expected = utils::read_bytes(path.join("expected.diff"))?;

        let old_path: std::path::PathBuf = old_path.components().skip(3).collect();
        let new_path: std::path::PathBuf = new_path.components().skip(3).collect();

        let old_path = buffer.insert(old_path.to_string_lossy().as_bytes())?;
        let new_path = buffer.insert(new_path.to_string_lossy().as_bytes())?;

        let options = diff::DiffOptions {
            strip_path_components: 0,
            ..Default::default()
        };

        let file = diff::diff_file(
            &buffer,
            old_path,
            new_path,
            old_body,
            new_body,
            &options,
            diff::DiffAlgorithm::default(),
        )?;
        let mut diff = diff::Diff::new(options);
        diff.add_file(file);

        let mut writer = diff::ChunkByteBufferWriter::new();
        diff.render(&mut writer.with_buffer(&buffer));

        assert_eq!(String::from_utf8(expected)?, String::from_utf8(writer.out)?);
    }

    Ok(())
}
