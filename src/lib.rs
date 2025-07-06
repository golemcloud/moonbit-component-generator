use anyhow::anyhow;
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use heck::ToLowerCamelCase;
use include_dir::{Dir, include_dir};
use lazy_static::lazy_static;
use log::{debug, info};
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Display;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use topologic::AcyclicDependencyGraph;
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::{PackageId, Resolve, WorldId};

static MOONBIT_CORE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/core");

#[derive(Default)]
struct MoonC {
    initialized: AtomicBool,
}

impl MoonC {
    pub fn run(&self, mut args: Vec<String>) -> anyhow::Result<()> {
        self.ensure_initialized()?;
        debug!("Running the MoonBit compiler with args: {}", args.join(" "));
        args.insert(0, "moonc".to_string());
        moonc_wasm::run_wasmoo(args)?;
        Ok(())
    }

    fn ensure_initialized(&self) -> anyhow::Result<()> {
        if !self.initialized.load(Ordering::Acquire) {
            debug!("Initializing V8...");
            moonc_wasm::initialize_v8()?;
            self.initialized.store(true, Ordering::Release);
        }
        Ok(())
    }
}

lazy_static! {
    static ref MOONC: MoonC = MoonC::default();
}

pub struct MoonBitComponent {
    dir: Utf8PathBuf,
    temp: Option<Utf8TempDir>,
    packages: BTreeMap<String, MoonBitPackage>,
    resolve: Option<Resolve>,
    world_id: Option<WorldId>,
    root_package_id: Option<PackageId>,
}

impl MoonBitComponent {
    /// Initializes a new MoonBit component that implements the given WIT interface.
    ///
    /// This step will create a temporary directory and generate MoonBit WIT bindings in it.
    pub fn empty_from_wit(
        wit: impl AsRef<str>,
        selected_world: Option<&str>,
    ) -> anyhow::Result<Self> {
        let temp_dir = Utf8TempDir::new()?;
        let dir = temp_dir.path().to_path_buf();

        info!("Creating MoonBit component in temporary directory: {dir}");

        let mut component = MoonBitComponent {
            dir,
            temp: Some(temp_dir),
            packages: BTreeMap::new(),
            resolve: None,
            world_id: None,
            root_package_id: None,
        };

        info!("Saving WIT package to {}/package.wit", component.wit_dir());
        std::fs::create_dir_all(component.wit_dir())?;
        std::fs::write(
            component.wit_dir().join("package.wit"),
            wit.as_ref().as_bytes(),
        )?;

        info!("Resolving WIT package");
        let mut resolve = Resolve::default();
        let (root_package_id, _) = resolve.push_dir(component.wit_dir())?;
        let world_id = resolve.select_world(root_package_id, selected_world)?;

        info!("Generating MoonBit WIT bindings");
        let mut wit_bindgen = wit_bindgen_moonbit::Opts {
            gen_dir: "gen".to_string(),
            ..Default::default()
        }
        .build();
        let mut bindgen_files = wit_bindgen_core::Files::default();
        wit_bindgen.generate(&resolve, world_id, &mut bindgen_files)?;

        for (name, contents) in bindgen_files.iter() {
            let dst = if let Some(stripped_name) = name.strip_prefix('/') {
                component.dir.join(stripped_name)
            } else {
                component.dir.join(name)
            };
            debug!("Writing binding file {dst}");

            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dst, contents)?;
        }

        component.extract_core()?;

        component.resolve = Some(resolve);
        component.world_id = Some(world_id);
        component.root_package_id = Some(root_package_id);
        Ok(component)
    }

    /// Disables cleaning up the temporary directory, for debugging purposes.
    pub fn disable_cleanup(&mut self) {
        if let Some(temp) = &mut self.temp {
            temp.disable_cleanup(true);
        }
    }

    /// Initializes a new MoonBit component from an existing directory with a valid 'wit' directory in it.
    ///
    /// The existing directory is expected to have the WIT bindings already generated.
    pub fn existing(path: &Utf8Path, selected_world: Option<&str>) -> anyhow::Result<Self> {
        let mut component = MoonBitComponent {
            dir: path.to_path_buf(),
            temp: None,
            packages: BTreeMap::new(),
            resolve: None,
            world_id: None,
            root_package_id: None,
        };
        component.extract_core()?;

        info!("Resolving WIT package");
        let mut resolve = Resolve::default();
        let (root_package_id, _) = resolve.push_dir(component.wit_dir())?;
        let world_id = resolve.select_world(root_package_id, selected_world)?;

        component.resolve = Some(resolve);
        component.world_id = Some(world_id);
        component.root_package_id = Some(root_package_id);

        Ok(component)
    }

    pub fn define_package(&mut self, package: MoonBitPackage) {
        debug!("Adding MoonBit package: {}", package.name);
        self.packages.insert(package.name.clone(), package);
    }

    pub fn write_world_stub(&self, moonbit_source: &str) -> anyhow::Result<()> {
        let world_name = self
            .resolve
            .as_ref()
            .and_then(|r| r.worlds.get(self.world_id?))
            .map(|w| w.name.to_string())
            .ok_or_else(|| anyhow::anyhow!("Could not find world"))?
            .to_lower_camel_case();
        let path = self
            .dir
            .join("gen")
            .join("world")
            .join(world_name)
            .join("stub.mbt");
        info!("Writing world stub to {}", path);
        std::fs::write(path, moonbit_source)?;
        Ok(())
    }

    pub fn build(&self, main_package_name: &str, target: &Utf8Path) -> anyhow::Result<()> {
        let sorted_packages = self.sorted_packages()?;
        debug!(
            "Package build order: {}",
            sorted_packages
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );

        for package in &sorted_packages {
            self.build_package(
                &package.mbt_files,
                &package.warning_control,
                &package.output,
                &package.name,
                &package.dependencies,
                &package.package_sources,
            )?
        }

        let mut core_files = vec![
            self.core_bundle_dir().join("abort").join("abort.core"),
            self.core_bundle_dir().join("core.core"),
        ];
        let mut package_sources = BTreeMap::new();

        for package in &sorted_packages {
            core_files.push(package.output.clone());
            for (name, source) in &package.package_sources {
                package_sources.insert(name.clone(), source.clone());
            }
        }
        package_sources.insert("moonbitlang/core".to_string(), self.core_dir());
        let package_sources: Vec<(String, Utf8PathBuf)> = package_sources.into_iter().collect();

        let main_package = self
            .packages
            .get(main_package_name)
            .ok_or_else(|| anyhow!(format!("Main package '{main_package_name}' not found")))?;
        let (_, main_package_source) = main_package
            .package_sources
            .iter()
            .find(|(name, _)| name == main_package_name)
            .ok_or_else(|| {
                anyhow!(format!(
                    "Main package sources '{main_package_name}' not found"
                ))
            })?;

        let main_package_json = self.dir.join(main_package_source).join("moon.pkg.json");
        let linker_config = Self::extract_wasm_linker_config(&main_package_json)?;

        self.link_core(
            &core_files,
            main_package_name,
            &main_package_json,
            &package_sources,
            &linker_config.export_memory_name,
            &linker_config.exports,
            linker_config.heap_start_address,
        )?;

        self.embed_wit()?;
        self.create_component(target)?;

        Ok(())
    }

    fn extract_core(&self) -> anyhow::Result<()> {
        let core_dir = self.core_dir();
        info!("Extracting MoonBit core to {core_dir}");
        std::fs::create_dir_all(&core_dir)?;
        MOONBIT_CORE.extract(&core_dir)?;
        Ok(())
    }

    fn build_package(
        &self,
        mbt_files: &[Utf8PathBuf],
        warning_control: &[WarningControl],
        output: &Utf8Path,
        package: &str,
        dependencies: &[(Utf8PathBuf, String)],
        package_sources: &[(String, Utf8PathBuf)],
    ) -> anyhow::Result<()> {
        info!("Building MoonBit package: {package}");

        let mut args = vec![
            "build-package".to_string(),
            "-error-format".to_string(),
            "json".to_string(),
        ];
        for file in mbt_files {
            let full_path = self.dir.join(file);
            args.push(full_path.to_string());
        }
        for w in warning_control {
            args.push("-w".to_string());
            args.push(w.to_string());
        }
        args.push("-o".to_string());
        args.push(self.dir.join(output).to_string());
        args.push("-pkg".to_string());
        args.push(package.to_string());
        args.push("-std-path".to_string());
        args.push(self.core_bundle_dir().to_string());
        for (dep_path, dep_name) in dependencies {
            args.push("-i".to_string());
            let full_path = self.dir.join(dep_path);
            args.push(format!("{full_path}:{dep_name}"));
        }
        self.add_package_sources(&mut args, package_sources);
        args.push("-target".to_string());
        args.push("wasm".to_string());

        MOONC.run(args)?;
        Ok(())
    }

    fn link_core(
        &self,
        core_files: &[Utf8PathBuf],
        main_package_name: &str,
        main_package_json: &Utf8Path,
        package_sources: &[(String, Utf8PathBuf)],
        exported_memory_name: &str,
        exported_functions: &[String],
        heap_start_address: usize,
    ) -> anyhow::Result<()> {
        info!("Linking MoonBit component");
        let mut args = vec!["link-core".to_string()];

        for file in core_files {
            let full_path = self.dir.join(file);
            args.push(full_path.to_string());
        }
        args.push("-main".to_string());
        args.push(main_package_name.to_string());
        args.push("-o".to_string());
        args.push(self.module_wasm().to_string());
        args.push("-pkg-config-path".to_string());
        args.push(self.dir.join(main_package_json).to_string());
        self.add_package_sources(&mut args, package_sources);
        args.push("-pkg-sources".to_string());
        args.push(format!("moonbitlang/core:{}", self.core_dir()));
        args.push("-target".to_string());
        args.push("wasm".to_string());
        args.push(format!(
            "-exported_functions={}",
            exported_functions.join(",")
        ));
        args.push("-export-memory-name".to_string());
        args.push(exported_memory_name.to_string());
        args.push("-heap-start-address".to_string());
        args.push(heap_start_address.to_string());

        MOONC.run(args)?;
        Ok(())
    }

    fn add_package_sources(
        &self,
        args: &mut Vec<String>,
        package_sources: &[(String, Utf8PathBuf)],
    ) {
        for (source_name, source_path) in package_sources {
            args.push("-pkg-sources".to_string());
            let full_path = self.dir.join(source_path);
            args.push(format!("{source_name}:{full_path}"));
        }
    }

    fn embed_wit(&self) -> anyhow::Result<()> {
        info!("Embedding WIT in the compiled MoonBit WASM module");

        // based on 'wasm-tools component embed'
        let resolve = self.resolve.as_ref().unwrap();
        let world = &self.world_id.unwrap();

        let mut wasm = std::fs::read(self.module_wasm())?;

        wit_component::embed_component_metadata(
            &mut wasm,
            &resolve,
            *world,
            StringEncoding::UTF16,
        )?;

        std::fs::write(self.module_with_embed_wasm(), wasm)?;

        Ok(())
    }

    fn create_component(&self, target: &Utf8Path) -> anyhow::Result<()> {
        info!("Creating the final WASM component at {}", target);

        let wasm = std::fs::read(self.module_with_embed_wasm())?;
        let mut encoder = ComponentEncoder::default()
            .validate(true)
            .reject_legacy_names(false)
            .merge_imports_based_on_semver(true)
            .realloc_via_memory_grow(false)
            .module(&wasm)?;

        let component = encoder.encode()?;

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, component)?;

        Ok(())
    }

    fn sorted_packages(&self) -> anyhow::Result<Vec<&MoonBitPackage>> {
        let mut graph = AcyclicDependencyGraph::new();
        let root_package = self.moonbit_root_package()?;

        for package in self.packages.values() {
            for (path, dep) in &package.dependencies {
                let path_components = path.components().map(|c| c.to_string()).collect::<Vec<_>>();
                let full_dep = if path_components.starts_with(&[
                    "target".to_string(),
                    "wasm".to_string(),
                    "release".to_string(),
                    "build".to_string(),
                ]) {
                    let relevant_path = &path_components[4..path_components.len() - 1];
                    format!("{}/{}", root_package, relevant_path.join("/"))
                } else {
                    format!("{}/{}", root_package, dep)
                };

                graph.depend_on(package.name.clone(), full_dep)?;
            }
        }

        let mut sorted = Vec::new();
        let mut names = HashSet::new();
        for layer in graph.get_forward_dependency_topological_layers() {
            for package_name in layer {
                sorted.push(&self.packages[&package_name]);
                names.insert(package_name);
            }
        }

        for package in self.packages.values() {
            if !names.contains(&package.name) {
                sorted.push(package);
            }
        }

        Ok(sorted)
    }

    fn wit_dir(&self) -> Utf8PathBuf {
        self.dir.join("wit")
    }

    fn core_dir(&self) -> Utf8PathBuf {
        self.dir.join("core")
    }

    fn core_bundle_dir(&self) -> Utf8PathBuf {
        self.dir
            .join("core")
            .join("target")
            .join("wasm")
            .join("release")
            .join("bundle")
    }

    fn module_wasm(&self) -> Utf8PathBuf {
        self.dir.join("target").join("module.wasm")
    }

    fn module_with_embed_wasm(&self) -> Utf8PathBuf {
        self.dir.join("target").join("module.embed.wasm")
    }

    fn extract_wasm_linker_config(
        package_json_path: &Utf8Path,
    ) -> anyhow::Result<WasmLinkerConfig> {
        debug!("Extracting Wasm linker config from {}", package_json_path);
        let json_str = std::fs::read_to_string(package_json_path)?;
        let pkg: PackageJsonWithWasmLinkerConfig = serde_json::from_str(&json_str)?;

        Ok(pkg.link.wasm)
    }

    fn moonbit_root_package(&self) -> anyhow::Result<String> {
        let root_package_id = self.root_package_id.as_ref().unwrap();
        let resolve = self.resolve.as_ref().unwrap();

        let root_package = resolve
            .packages
            .get(*root_package_id)
            .ok_or_else(|| anyhow!("Root package not found"))?;
        Ok(format!(
            "{}/{}",
            root_package.name.namespace, root_package.name.name
        ))
    }
}

#[derive(Debug, Deserialize)]
struct PackageJsonWithWasmLinkerConfig {
    link: LinkConfig,
}

#[derive(Debug, Deserialize)]
struct LinkConfig {
    wasm: WasmLinkerConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct WasmLinkerConfig {
    export_memory_name: String,
    exports: Vec<String>,
    heap_start_address: usize,
}

pub struct MoonBitPackage {
    pub name: String,
    pub mbt_files: Vec<Utf8PathBuf>,
    pub warning_control: Vec<WarningControl>,
    pub output: Utf8PathBuf,
    pub dependencies: Vec<(Utf8PathBuf, String)>,
    pub package_sources: Vec<(String, Utf8PathBuf)>,
}

pub enum Warning {
    Specific(u16),
    Range(Range<u16>),
}

impl Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Warning::Specific(code) => write!(f, "{code}"),
            Warning::Range(range) => write!(f, "{}..{}", range.start, range.end),
        }
    }
}

pub enum WarningControl {
    Enable(Warning),
    Disable(Warning),
    EnableAsError(Warning),
}

impl Display for WarningControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarningControl::Enable(code) => write!(f, "+{code}"),
            WarningControl::Disable(code) => write!(f, "-{code}"),
            WarningControl::EnableAsError(code) => write!(f, "@{code}"),
        }
    }
}

// pub fn test() -> anyhow::Result<()> {
//     moonc_wasm::initialize_v8()?;
//     moonc_wasm::run_wasmoo(vec!["--help".to_string()])?;
//     Ok(())
// }

#[cfg(test)]
test_r::enable!();

#[cfg(test)]
mod tests {
    use crate::{MoonBitComponent, MoonBitPackage, Warning, WarningControl};
    use camino::Utf8Path;
    use indoc::indoc;
    use log::LevelFilter;
    use pretty_env_logger::env_logger::WriteStyle;
    use std::fmt::Write;
    use test_r::{test, test_dep};
    use wit_bindgen_core::uwriteln;

    struct Trace;

    #[test_dep]
    fn initialize_trace() -> Trace {
        pretty_env_logger::formatted_builder()
            .filter_level(LevelFilter::Debug)
            .write_style(WriteStyle::Always)
            .init();
        Trace
    }

    #[test]
    fn generate_get_script_component(_trace: &Trace) -> anyhow::Result<()> {
        let mut component = MoonBitComponent::empty_from_wit(
            r#"
            package golem:script-source;

            world script-source {
                export get-script: func() -> string;
            }
            "#,
            Some("script-source"),
        )?;
        component.disable_cleanup();

        // TODO: this can be automated in from_wit mode by getting the root package name from the WIT resolve
        component.define_package(MoonBitPackage {
            name: "golem/script-source/ffi".to_string(),
            mbt_files: vec![Utf8Path::new("ffi/top.mbt").to_path_buf()],
            warning_control: vec![WarningControl::Disable(Warning::Specific(44))],
            output: Utf8Path::new("target/wasm/release/build/ffi/ffi.core").to_path_buf(), // TODO: this can be generated from the package name
            dependencies: vec![],
            package_sources: vec![
                // TODO these too?
                (
                    "golem/script-source/ffi".to_string(),
                    Utf8Path::new("ffi").to_path_buf(),
                ),
            ],
        });

        // TODO: this too, using the world name
        component.define_package(MoonBitPackage {
            name: "golem/script-source/gen/world/scriptSource".to_string(),
            mbt_files: vec![Utf8Path::new("gen/world/scriptSource/stub.mbt").to_path_buf()],
            warning_control: vec![],
            output: Utf8Path::new(
                "target/wasm/release/build/gen/world/scriptSource/scriptSource.core",
            )
            .to_path_buf(),
            dependencies: vec![],
            package_sources: vec![(
                "golem/script-source/gen/world/scriptSource".to_string(),
                Utf8Path::new("gen/world/scriptSource").to_path_buf(),
            )],
        });

        component.define_package(MoonBitPackage {
            name: "golem/script-source/gen".to_string(),
            mbt_files: vec![
                Utf8Path::new("gen/world_script_source_export.mbt").to_path_buf(),
                Utf8Path::new("gen/ffi.mbt").to_path_buf(),
            ],
            warning_control: vec![],
            output: Utf8Path::new("target/wasm/release/build/gen/gen.core").to_path_buf(),
            dependencies: vec![
                (
                    Utf8Path::new("target/wasm/release/build/ffi/ffi.mi").to_path_buf(),
                    "ffi".to_string(),
                ),
                (
                    Utf8Path::new(
                        "target/wasm/release/build/gen/world/scriptSource/scriptSource.mi",
                    )
                    .to_path_buf(),
                    "scriptSource".to_string(),
                ),
            ],
            package_sources: vec![(
                "golem/script-source/gen".to_string(),
                Utf8Path::new("gen").to_path_buf(),
            )],
        });

        let script = indoc! {
            r#"export function hello() {
                 return "Hello, World!";
            }"#
        };

        let mut stub_mbt = String::new();
        uwriteln!(stub_mbt, "// Generated by `moonbit-component-generator`");
        uwriteln!(stub_mbt, "");
        uwriteln!(stub_mbt, "pub fn get_script() -> String {{");
        for line in script.lines() {
            uwriteln!(stub_mbt, "    #|{line}");
        }
        uwriteln!(stub_mbt, "}}");

        component.write_world_stub(&stub_mbt)?;

        component.build(
            "golem/script-source/gen",
            Utf8Path::new("target/test-output/generate_get_script_component.wasm"),
        )?;

        Ok(())
    }
}
