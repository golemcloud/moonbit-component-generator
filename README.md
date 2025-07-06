# moonbit-component-generator

Rust library embedding the MoonBit compiler (compiled to WASM, executed on V8) and the MoonBit core library and various
tools from the WASM ecosystem to programmatically generate MoonBit source code and compile it to WebAssembly Components.

## Development

- `moonc_wasm` is a cloned and patched version of https://github.com/moonbitlang/moonc_wasm to make it library crate.
- `core` is a git submodule containing the MoonBit core library (https://github.com/moonbitlang/core)

To update and build the MoonBit core library:

```
git submodule update --recursive
moon bundle --target wasm
```

The bundled core library is included in the compiled crate using the `include_dir!` macro.
