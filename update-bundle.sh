#!/bin/bash

pushd core
moon bundle --target wasm
popd

rm -rf bundled-core
cp -rv core bundled-core
rm -rf bundled-core/.git
