# Version Control Tools

A collection of command-line (and TUI) utilities and supporting libraries to make working
with version control easier. The focus is on code review on GitHub.
There is a [separate README for the `git-inbox` and `git-review` tools](./git-forge-tui/README.md).

Written in Rust and [licensed](./LICENSE) under a mixture of MIT and GPL licenses.

## Crates

* `diff-modulo-base`: Command-line tools and library for computing diffs and diff-of-diffs.
* `git-forge-tui`: TUI tools and library for interacting with reviews and notifications on GitHub.
* `vctools-utils`: A library containing a small number of random utility functions.
* `vctuik`: An immediate mode TUI toolkit that layers composable input handling
  on top of [ratatui](https://ratatui.rs/).
* `vctuik-unsafe-internals`: A library containing helpers for `vctuik` written with unsafe Rust code.
  There is no use of `unsafe` outside of this crate.

## Contributing

Contributions are welcome, just open a pull request here on GitHub.
You may want to take a look at the [TODOs](./TODO.md).
