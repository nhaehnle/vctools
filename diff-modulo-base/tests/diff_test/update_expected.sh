#!/bin/sh
#
# Update `expected.diff` files.

path=$(dirname $0)
echo "Updating expected.diff files in $path ..."

cd $path
for entry in *; do
    if [ -d "$entry" ]; then
        cargo run --example devtool -- diff \
            $entry/old \
            $entry/new \
            > $entry/expected.diff
    fi
done
