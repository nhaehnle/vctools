#!/bin/sh
#
# Update `test-expected` files.

path=$(dirname $0)
echo "Updating test-expected files in $path ..."

cd $path
for entry in *; do
    if [ -d "$entry" ]; then
        cargo run --example devtool -- git-diff-modulo-base \
            --mock-data $entry \
            $(cat $entry/test-cmdline) \
            > $entry/test-expected
    fi
done
