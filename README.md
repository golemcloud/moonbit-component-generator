# moonbit-component-generator

Rust library embedding the MoonBit compiler (compiled to WASM, executed on V8) and the MoonBit core library and various
tools from the WASM ecosystem to programmatically generate MoonBit source code and compile it to WebAssembly Components.

The crate is fully self-contained, does not require any external dependencies.
Some example use cases are exposed by crate features:

- `get-script`: generates a WASM component with a single exported function `get-script` that returns an arbitrary string
  embedded in the component. An example use case for this can be to compose JavaScript scripts with a precompiled WASM
  JS engine.
- `typed-config`: takes a simple WIT interface with functions getting "typed configuration", and generates a WASM
  component that implements this interface using either the WASI environment or the WASI config APIs.

## Use cases
The crate implements the general machinery for generating WASM components from MoonBit source, and it also exports a few example use cases:

### "Get Script" component
The simplest example generates a WASM component that exports a single function `get-script` which returns a string.
This can be used to attach dynamic content to a statically compiled WASM component via composition,
for example providing user-defined JavaScript code to a precompiled WASM JavaScript engine.

To use this generator, just provide the script contents and the target WASM path:

```rust
use moonbit_component_generator::get_script::generate_get_script_component;

fn main() {
    let script = "console.log('Hello, world!');";
    let wasm_path = "output/get_script.wasm";
    generate_get_script_component(script, wasm_path).expect("Failed to generate get-script component");
}
```

### "Typed Config" component
The "Typed Config" component generator inspects a given WIT package and generates a WASM component that implements it
using either the WASI environment API or the WASI config APIs. This can be used to have a statically typed configuration
API provided for WASM components of any language.

The current implementation only supports string configuration values, but it can be extended to support more types including
records and variants, as the code generator has access to the resolved WIT interface.

To use this generator, provide the WIT package path and the target WASM path:

```rust
use moonbit_component_generator::typed_config::generate_typed_config_component;

fn main() {
  let wit = r#"
                package example:typed-config;

                interface config {
                    username: func() -> string;
                    password: func() -> string;
                    server: func() -> string;
                }

                world example-config {
                    export config;
                }
            "#;
  let wasm_path = "output/typed_config.wasm";
  generate_typed_config_component(wit, wasm_path).expect("Failed to generate typed config component");
}

```

## Development

- `moonc_wasm` is a cloned and patched version of https://github.com/moonbitlang/moonc_wasm to make it library crate.
- `core` is a git submodule containing the MoonBit core library (https://github.com/moonbitlang/core)
- `bundled-core` is the MoonBit core library (source and compiled for wasm), included in this repository to avoid users of the crate from having to build `core` themselves.

### System Requirements

#### Debian/Ubuntu
The following system packages are required:
```bash
# Essential build tools and C compiler for Rust build dependencies
sudo apt-get update && sudo apt-get install -y build-essential

# Optional but recommended: curl for downloading tools
sudo apt-get install -y curl

# Git for submodule management
sudo apt-get install -y git
```

**Note**: The project has been tested on Debian 12 (Bookworm) with kernel 6.1.0-38-amd64 and works correctly with the standard Debian package repositories.

### Development Setup

1. **Install mise** (development environment manager):
```bash
# Install mise if not already present
curl https://mise.run | sh
```

2. **Install development tools** via mise:
```bash
# This installs Rust stable toolchain as configured in .mise.toml
mise install
```

3. **Install MoonBit** (version 0.6.24):
```bash
curl -fsSL https://cli.moonbitlang.com/install/unix.sh | bash -s -- 0.6.24
# Add to PATH (the installer does this automatically for new shells)
export PATH="$HOME/.moon/bin:$PATH"
```

4. **Initialize git submodules** and build the core library:
```bash
git submodule update --init --recursive
cd core && moon bundle --target wasm && cd ..
```

5. **Install test dependencies**:
```bash
# Install cargo-binstall for faster binary installation
curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# Install wasmtime-cli with required features
cargo binstall --force --locked wasmtime-cli@33.0.0 -y
```

### Building and Testing

```bash
# Format check
cargo fmt -- --check

# Lint check
cargo clippy -- -Dwarnings

# Build all features and targets
cargo build --all-features --all-targets

# Run tests (requires wasmtime-cli)
cargo test -p moonbit-component-generator -- --nocapture --test-threads=1
```

### Updating the MoonBit Core Library

To update and rebuild the MoonBit core library bundle:

```bash
git submodule update --recursive
./update-bundle.sh
```

The bundled core library is included in the compiled crate using the `include_dir!` macro.
It is also pushed into the repository (`bundled-core` directory) to avoid users of the crate from having to build the MoonBit core themselves as part
of the Rust build process.

### Notes

- Running tests requires `wasmtime-cli` version 33.0.0 with the following features enabled:
  - `component-model`
  - `wasi-config`
- The MoonBit compiler runs in WASM on V8, so the first compilation in a session may take longer due to V8 initialization
