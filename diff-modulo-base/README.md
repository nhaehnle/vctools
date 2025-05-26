# Diff modulo Base

You're reviewing a large pull request on a project where folks care about a
clean commit history. The author has just force-pushed a new version of the
pull request and you want to see only what has changed. But, oh no! They rebased
on a more recent version of the destination branch, and now the diff is full of
all sorts of unrelated changes. And how can you check if rebase conflicts were
resolved appropriately?

Diff modulo Base comes to the rescue!

## Building and Installing

Use `cargo build` or `cargo install` with a reasonably recent version of stable
Rust. Two binaries are installed:

* `diff-modulo-base` operates on plain diff files to make the core diffing
  algorithm available
* `git-diff-modulo-base` directly compares Git commits or branches

## Usage

This utility allows you to compare the relevant changes of two versions of a
rebased branch given three input diffs: two *base* diffs that show the changes
since the respective merge bases and a *target* diff between the branches you
are actually interested in.

As such, it is very similar to (and actually builds on) `git range-diff`. The
difference is that the resulting output is arguably much more readable.

**Example:** Let's say your Git history looks like this, where `main` is the
main development branch and `feature` is some feature branch that has just been
rebased:
```
  o--o--A--o--o-- ... -- o -- B -- o -- o   (origin/main)
         \                     \
          \                     o -- o -- o -- D (origin/feature)
           \
            o -- o -- o -- C (origin/feature@{1})
```
Running
```bash
git diff-modulo base origin/main origin/feature@{1} origin/feature
```
produces output similar to `git range-diff`. However, instead of producing a
"diff of diffs", the output is the `origin/feature@{1}..origin/feature` diff
with noise from unrelated changes in `A..B` filtered out.

The output of the tool is meant to help in the reviewing changes made to pull
requests. In particular, it is meant to help answer two questions:

* What changes are real changes in the pull request, as opposed to noise that
  resulted from rebasing?
* Have there been changes in the merge base that are relevant to the pull
  request? For example, has a change in the base version been accidentally
  dropped during rebase?

The output is technically an interleaving of two diffs:

* The (reduced) target diff, which is the diff you'll be most interested in
  (diff between `C` and `D` in the example).
* Relevant parts of the base diff (diff between `A` and `B` in the example),
  prefixed with hash (`#`) characters.

There may be lines starting with `<` or `>`. Those are changes that are deemed
"unimportant" by the tool. A change in the target diff that is entirely caused
by changes in the base version is considered unimportant unless it is likely to
have been involved in textual conflicts during rebase. Similarly, a change in
the base version is considered unimportant if it is not near any changes in the
base diffs.

### Advanced git usage

Make sure to look at the available command-line options to see some alternative
modes of usage. For example, it is possible to compare two individual commits.
This occasionally comes up after cherry-picking:
```bash
git diff-modulo-base ${original-commit-hash} ${cherry-picked-commit-hash}
```
If you're working with repositories hosted on GitHub, it may be convenient to
automatically fetch pull request branches by adding a line such as the
following to the relevant `remote` section of your local repository's
`.git/config`:
```
fetch = +refs/pull/*:refs/remotes/origin/pull/*
```

### Usage on raw diffs

Very similar output can also be obtained by invoking the underlying (not
Git-aware) `diff-modulo-base` with the relevant diffs:
```bash
diff-modulo-base <(git diff A..origin/feature@{1}) <(git diff B..origin/feature) <(git diff origin/feature@{1}..origin/feature)
```
This produces the same kind of diff output, albeit not split according to
individual commits on the feature branch. The same effect can be achieved using
the `--combined` flag of `git diff-modulo-base`.

## Details

`diff-modulo-base` expects standard Git-style diffs as input and works
standalone entirely based on the input diffs. In particular, the tool does not
attempt to open any of the files mentioned in the diffs.

The `diff-modulo-base` command-line utility is built on a reusable library crate
that implements the underlying algorithm, including the required diff parsing
and writing.

Diffs are treated as "don't care about extended characters ASCII". That is,
diffs are required to use an encoding where all ASCII characters use their
standard, single-byte ASCII encoding, and any non-ASCII characters are encoded
using bytes that do not correspond to ASCII characters. Just use UTF-8 and
you'll be fine.

The underlying algorithm is, at a high level:

1. Parse the base and target diffs to obtain base and target sequences of
   "chunks". Chunks are either file headers or diff "hunks".

2. For hunks in the target diff, refer to the base diffs to check whether the
   lines were changed relative to the corresponding base versions. If they were
   not, mark them as context changes (lines that will be output with a `<` or
   `>` prefix) and drop hunks or files that are entirely unmodified or context
   changes.

The exact details of this algorithm are subject to change.

## Contributing and License

Please integrate this feature into as many development tools as you can find.
To that end, the project is released under the MIT license.

## To do

* Smarter alignment of "base" and "target" changes: remap line numbers using
  the old and new diffs
