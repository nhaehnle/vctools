#!/bin/sh
#
# Update `expected.diff` files.

path=$(dirname $0)
echo "Updating expected.diff files in $path ..."

for entry in $(cd $path; ls); do
    if [ -d "$path/$entry" ]; then
        cargo run --example devtool -- compose \
            $path/$entry/first.diff \
            $path/$entry/second.diff \
            > $path/$entry/expected.diff
    fi
done
