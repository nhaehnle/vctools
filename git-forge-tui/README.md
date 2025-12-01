# git-forge-tui

Command-line tools for reviewing GitHub pull requests with intelligent diff
visualization using "diff modulo base" technology.

## Overview

`git-inbox` is a work-in-progress TUI client for GitHub notifications. It aims
to be a fast way to browse notifications and the corresponding reviews and
issues.

`git-review` is designed to make pull request reviews more efficient by showing
only the meaningful changes between your last review and the current state of a
pull request. It integrates with GitHub's API to fetch pull request information
and uses the "diff modulo base" algorithm to filter out noise from rebases and
merges.

## Key Features

- **GitHub Integration**: Fetches pull request data directly from the GitHub API
  and computes diffs locally from commits fetched via `git fetch`
- **Review Context**: Tracks your previous reviews to show incremental changes
- **Smart Diff Visualization**: Uses diff-modulo-base algorithm to show only relevant changes
- **Interactive TUI**: Terminal-based interface for navigation
- **Range Diff Integration**: Shows commit-by-commit changes for teams that
  carefully author multiple commit per pull request

## Installation

```bash
cargo install --path .
```

Or just build from source without installing:

```bash
cargo build --release
```

## Configuration

Create a configuration file at `~/.config/vctools/github.toml`:

```toml
[[hosts]]
host="github.com"
api="https://api.github.com"
user="<your-github-username>"
token="ghp_<your-github-token>"

# Optional aliases for the host name.
alias=["gh"]
```

Multiple hosts can be specified, which is useful if you are working with GitHub Enterprise
or multiple GitHub usernames.

The `host` field (or the aliases) must match the host that is used in the remotes set up for your
Git working directories.

For GitHub Enterprise installations, the API URL is typically of the form
https://ghe.example/api/v3.

Register repositories (only required for `git-inbox`) at `~/.config/vctools/repositories.toml`:

```toml
[[repository]]
path="/path/to/first/repository"

[[repository]]
path="/path/to/second/repository"
```

### GitHub Token Setup

1. Go to GitHub Settings → Developer settings → Personal access tokens
2. Generate a new token and add it to your configuration file
   - You can use a classic token for simplicity (no special rights needed in
     that case), but feel free to use a fine-grained token if you're feeling
     paranoid

## Usage

The most basic usage is, from within a Git working directory

```bash
git-review <remote> <pull-request-number> [OPTIONS]
```

## Key bindings

* `q`: quit
* `/`: search
* `n`: find next
* `N`: find previous
* Up / down / page up / page down: navigate vertically
* Alt+Up / down / page up / page down: scroll vertically
* Left / right: navigate horizontally / fold and unfold
* `g`: go to top
* `G`: go to bottom
* `C`: toggle combined diff vs. range diff
* `d`: cycle through diff styles (unified / only old / only new)
* `e`: mark a notification as "done"
* `M`: unsubscribe from a thread

## How It Works

1. **Fetch PR Data**: Connects to GitHub API to get pull request information including:
   - Current head commit
   - Target branch
   - Your previous reviews (if any)

2. **Intelligent Diff**: Uses the diff-modulo-base algorithm to generate a meaningful diff:
   - If you've reviewed before: shows changes since your last review
   - If first review: shows changes from the merge base
   - Filters out noise from rebases and conflict resolutions

3. **Interactive Display**: Shows the diff in a terminal UI with:
   - Navigation controls
   - Commit-by-commit breakdown
   - File and hunk navigation

## License

Licensed under GPL 3.0 or later.
