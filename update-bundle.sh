#!/bin/bash

MOON=${1:-moon}

pushd core
$MOON bundle --target wasm
popd

rm -rf bundled-core
cp -rv core bundled-core
rm -rf bundled-core/.git
sed -i '' '\|target/|d' bundled-core/.gitignore
