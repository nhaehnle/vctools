# Tests

Most of the tests in this project are data driven. Rust tests such as
[dmb_test](./dmb_test.rs) iterate over corresponding subdirectories, in this
case in the [tests/dmb_test](./dmb_test/) directory.

Each subdirectory contains input and expected output files.

Use `updated_expected.sh` scripts to update the expected output files
automatically in bulk. Make sure to verify the result using `git diff`.
This means reading and interpreting a diff of diffs, which is confusing but
necessary.
