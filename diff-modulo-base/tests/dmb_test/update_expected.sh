#!/bin/sh
#
# Update `expected.diff` files.

path=$(dirname $0)
echo "Updating expected.diff files in $path ..."

for entry in $(cd $path; ls); do
    if [ -d "$path/$entry" ]; then
        cargo run --bin diff-modulo-base \
            $path/$entry/old.diff \
            $path/$entry/new.diff \
            $path/$entry/target.diff \
            > $path/$entry/expected.diff
    fi
done

