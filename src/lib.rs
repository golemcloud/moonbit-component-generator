use anyhow::{Context, anyhow};
use camino::{Utf8Path, Utf8PathBuf};
use camino_tempfile::Utf8TempDir;
use heck::{ToLowerCamelCase, ToSnakeCase};
use include_dir::{Dir, include_dir};
use lazy_static::lazy_static;
use log::debug;
use log::info;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Display;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use topologic::AcyclicDependencyGraph;
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::{PackageId, PackageName, Resolve, WorldId};

mod moonc_wasm;

/// An example generator that embeds and exports a script (arbitrary string) into a MoonBit component.
#[cfg(feature = "get-script")]
pub mod get_script;

/// An example generator that implements a simple typed configuration interface defined in WIT
#[cfg(feature = "typed-config")]
pub mod typed_config;

static MOONBIT_CORE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/bundled-core");

#[derive(Default)]
struct MoonC {
    initialized: AtomicBool,
}

impl MoonC {
    pub fn run(&self, mut args: Vec<String>) -> anyhow::Result<()> {
        self.ensure_initialized()?;
        debug!("Running the MoonBit compiler with args: {}", args.join(" "));
        args.insert(0, "moonc".to_string());
        moonc_wasm::run_wasmoo(args).context("Running the MoonBit compiler")?;
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
        let temp_dir = Utf8TempDir::new().context("Creating temporary directory")?;
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
        std::fs::create_dir_all(component.wit_dir()).context("Creating WIT package directory")?;
        std::fs::write(
            component.wit_dir().join("package.wit"),
            wit.as_ref().as_bytes(),
        )?;

        info!("Resolving WIT package");
        let mut resolve = Resolve::default();
        let (root_package_id, _) = resolve
            .push_dir(component.wit_dir())
            .context("Resolving WIT packages")?;
        let world_id = resolve
            .select_world(root_package_id, selected_world)
            .context("Selecting the WIT world")?;

        info!("Generating MoonBit WIT bindings");
        let mut wit_bindgen = wit_bindgen_moonbit::Opts {
            gen_dir: "gen".to_string(),
            derive_eq: true,
            derive_show: true,
            ..Default::default()
        }
        .build();
        let mut bindgen_files = wit_bindgen_core::Files::default();
        wit_bindgen
            .generate(&resolve, world_id, &mut bindgen_files)
            .context("Generating MoonBit WIT bindings")?;

        for (name, contents) in bindgen_files.iter() {
            let dst = if let Some(stripped_name) = name.strip_prefix('/') {
                component.dir.join(stripped_name)
            } else {
                component.dir.join(name)
            };
            debug!("Writing binding file {dst}");

            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)
                    .context("Creating directory for generated MoonBit WIT bindings")?;
            }
            std::fs::write(&dst, contents).context("Writing generated MoonBit WIT bindings")?;
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
        let (root_package_id, _) = resolve
            .push_dir(component.wit_dir())
            .context("Resolving WIT package")?;
        let world_id = resolve
            .select_world(root_package_id, selected_world)
            .context("Selecting WIT world")?;

        component.resolve = Some(resolve);
        component.world_id = Some(world_id);
        component.root_package_id = Some(root_package_id);

        Ok(component)
    }

    /// Defines the MoonBit packages implementing the WIT bindings
    pub fn define_bindgen_packages(&mut self) -> anyhow::Result<()> {
        let moonbit_root_package = self.moonbit_root_package()?;
        let world_name = self.world_name()?;
        let world_snake = world_name.to_snake_case();

        let imported_interfaces = self.get_imported_interfaces()?;
        let exported_interfaces = self.get_exported_interfaces()?;

        debug!("Imported interfaces: {imported_interfaces:?}");
        debug!("Exported interfaces: {exported_interfaces:?}");

        let mut gen_dependencies = Vec::new();
        let mut gen_mbt_files = Vec::new();

        self.define_package(MoonBitPackage {
            name: format!("{moonbit_root_package}/ffi"),
            mbt_files: vec![Utf8Path::new("ffi").join("top.mbt")],
            warning_control: vec![WarningControl::Disable(Warning::Specific(44))],
            output: Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("ffi")
                .join("ffi.core"),
            dependencies: vec![],
            package_sources: vec![(
                format!("{moonbit_root_package}/ffi"),
                Utf8Path::new("ffi").to_path_buf(),
            )],
        });
        let ffi_dep = (
            Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("ffi")
                .join("ffi.mi"),
            "ffi".to_string(),
        );
        gen_dependencies.push(ffi_dep.clone());

        self.define_package(MoonBitPackage {
            name: format!("{moonbit_root_package}/gen/world/{world_name}"),
            mbt_files: vec![
                Utf8Path::new("gen")
                    .join("world")
                    .join(&world_name)
                    .join("stub.mbt"),
            ],
            warning_control: vec![],
            output: Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("gen")
                .join("world")
                .join(&world_name)
                .join(format!("{world_name}.core")),

            dependencies: vec![],
            package_sources: vec![(
                format!("{moonbit_root_package}/gen/world/{world_name}"),
                Utf8Path::new("gen").join("world").join(&world_name),
            )],
        });
        gen_dependencies.push((
            Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("gen")
                .join("world")
                .join(&world_name)
                .join(format!("{world_name}.mi")),
            world_name.clone(),
        ));
        gen_mbt_files.push(Utf8Path::new("gen").join(format!("world_{world_snake}_export.mbt")));

        for (package_name, interface_name) in &imported_interfaces {
            let pkg_namespace = package_name.namespace.to_snake_case();
            let pkg_name = package_name.name.to_snake_case();
            let interface_name = interface_name.to_lower_camel_case();

            let name = format!(
                "{moonbit_root_package}/interface/{pkg_namespace}/{pkg_name}/{interface_name}"
            );
            let src = Utf8Path::new("interface")
                .join(&pkg_namespace)
                .join(&pkg_name)
                .join(&interface_name);
            let output = Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("interface")
                .join(&pkg_namespace)
                .join(&pkg_name)
                .join(&interface_name);
            self.define_package(MoonBitPackage {
                name: name.clone(),
                mbt_files: vec![src.join("top.mbt"), src.join("ffi.mbt")],
                warning_control: vec![],
                output: output.join(format!("{interface_name}.core")),
                dependencies: vec![ffi_dep.clone()],
                package_sources: vec![(name, src)],
            });
        }

        for (package_name, interface_name) in &exported_interfaces {
            let pkg_namespace = package_name.namespace.to_snake_case();
            let pkg_name = package_name.name.to_snake_case();
            let interface_name = interface_name.to_lower_camel_case();
            let snake_interface_name = interface_name.to_snake_case();

            let name = format!(
                "{moonbit_root_package}/gen/interface/{pkg_namespace}/{pkg_name}/{interface_name}"
            );
            let src = Utf8Path::new("gen")
                .join("interface")
                .join(&pkg_namespace)
                .join(&pkg_name)
                .join(&interface_name);
            let output = Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("gen")
                .join("interface")
                .join(&pkg_namespace)
                .join(&pkg_name)
                .join(&interface_name);
            self.define_package(MoonBitPackage {
                name: name.clone(),
                mbt_files: vec![src.join("top.mbt"), src.join("stub.mbt")],
                warning_control: vec![],
                output: output.join(format!("{interface_name}.core")),

                dependencies: vec![],
                package_sources: vec![(name, src)],
            });
            gen_dependencies.push((
                output.join(format!("{interface_name}.mi")),
                interface_name.clone(),
            ));
            gen_mbt_files.push(Utf8Path::new("gen").join(format!(
                "gen_interface_{pkg_namespace}_{pkg_name}_{snake_interface_name}_export.mbt"
            )));
        }

        gen_mbt_files.push(Utf8Path::new("gen").join("ffi.mbt"));
        self.define_package(MoonBitPackage {
            name: format!("{moonbit_root_package}/gen"),
            mbt_files: gen_mbt_files,
            warning_control: vec![],
            output: Utf8Path::new("target")
                .join("wasm")
                .join("release")
                .join("build")
                .join("gen")
                .join("gen.core"),
            dependencies: gen_dependencies,
            package_sources: vec![(
                format!("{moonbit_root_package}/gen"),
                Utf8Path::new("gen").to_path_buf(),
            )],
        });

        Ok(())
    }

    pub fn set_warning_control(
        &mut self,
        package_name: &str,
        warning_control: Vec<WarningControl>,
    ) -> anyhow::Result<()> {
        let package = self
            .packages
            .get_mut(package_name)
            .ok_or_else(|| anyhow::anyhow!("Package '{package_name}' not found"))?;
        package.warning_control = warning_control;
        Ok(())
    }

    /// Defines a custom MoonBit package
    pub fn define_package(&mut self, package: MoonBitPackage) {
        debug!("Adding MoonBit package: {}", package.name);
        self.packages.insert(package.name.clone(), package);
    }

    /// Defines an additional dependency for a MoonBit package previously added with `define_package`.
    pub fn add_dependency(
        &mut self,
        package_name: &str,
        mi_path: &Utf8Path,
        alias: &str,
    ) -> anyhow::Result<()> {
        debug!("Adding dependency: {package_name} ({mi_path}) as {alias}");
        let package = self
            .packages
            .get_mut(package_name)
            .ok_or_else(|| anyhow::anyhow!("Package '{package_name}' not found"))?;
        package
            .dependencies
            .push((Utf8PathBuf::from(mi_path), alias.to_string()));
        Ok(())
    }

    pub fn write_file(&self, relative_path: &Utf8Path, contents: &str) -> anyhow::Result<()> {
        let path = self.dir.join(relative_path);
        info!("Writing file: {path:?}");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("Creating directory for generated file")?;
        }
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Writes the top level export stub for the selected world
    pub fn write_world_stub(&self, moonbit_source: &str) -> anyhow::Result<()> {
        let world_name = self.world_name()?;
        let path = self
            .dir
            .join("gen")
            .join("world")
            .join(world_name)
            .join("stub.mbt");
        info!("Writing world stub to {path}");
        std::fs::write(path, moonbit_source)?;
        Ok(())
    }

    /// Writes the interface stub for a given package and interface name.
    pub fn write_interface_stub(
        &self,
        package_name: &PackageName,
        interface_name: &str,
        moonbit_source: &str,
    ) -> anyhow::Result<()> {
        let package_name_snake = package_name.name.to_snake_case();
        let package_namespace_snake = package_name.namespace.to_snake_case();
        let path = self
            .dir
            .join("gen")
            .join("interface")
            .join(package_namespace_snake)
            .join(package_name_snake)
            .join(interface_name.to_lower_camel_case())
            .join("stub.mbt");
        info!("Writing interface stub to {path}");
        std::fs::create_dir_all(path.parent().unwrap())
            .context("Creating directory for interface stub")?;
        std::fs::write(path, moonbit_source).context("Writing interface stub")?;
        Ok(())
    }

    /// Writes the MoonBit package JSON file for a given exported package and interface name.
    pub fn write_interface_package_json(
        &self,
        package_name: &PackageName,
        interface_name: &str,
        json: serde_json::Value,
    ) -> anyhow::Result<()> {
        let package_name_snake = package_name.name.to_snake_case();
        let package_namespace_snake = package_name.namespace.to_snake_case();
        let path = self
            .dir
            .join("gen")
            .join("interface")
            .join(package_namespace_snake)
            .join(package_name_snake)
            .join(interface_name.to_lower_camel_case())
            .join("moon.pkg.json");
        info!("Writing interface definition to {path}");
        std::fs::create_dir_all(path.parent().unwrap())
            .context("Creating directory for interface definition")?;
        std::fs::write(
            path,
            serde_json::to_string_pretty(&json).context("Writing interface definition")?,
        )?;
        Ok(())
    }

    /// Builds the MoonBit component, compiling all packages and linking them together into a
    /// final WASM component.
    ///
    /// The `main_package_name` is optional, if it is not provided, the binding generator's `gen` package will be used as the main package.
    pub fn build(&self, main_package_name: Option<&str>, target: &Utf8Path) -> anyhow::Result<()> {
        let main_package_name = match main_package_name {
            Some(name) => name.to_string(),
            None => {
                let root_package = self.moonbit_root_package()?;
                format!("{root_package}/gen")
            }
        };

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
            )
            .context(format!("Building package {}", package.name))?;
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
            .get(&main_package_name)
            .ok_or_else(|| anyhow!(format!("Main package '{main_package_name}' not found")))?;
        let (_, main_package_source) = main_package
            .package_sources
            .iter()
            .find(|(name, _)| name == &main_package_name)
            .ok_or_else(|| {
                anyhow!(format!(
                    "Main package sources '{main_package_name}' not found"
                ))
            })?;

        let main_package_json = self.dir.join(main_package_source).join("moon.pkg.json");
        let linker_config = Self::extract_wasm_linker_config(&main_package_json)
            .context("Extracting linker config")?;

        self.link_core(
            &core_files,
            &main_package_name,
            &main_package_json,
            &package_sources,
            &linker_config.export_memory_name,
            &linker_config.exports,
            linker_config.heap_start_address,
        )
        .context("Linking")?;

        self.embed_wit().context("Embedding WIT")?;
        self.create_component(target)
            .context("Creating component")?;

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

        let mut args = vec!["build-package".to_string()];
        for file in mbt_files {
            let full_path = self.dir.join(file);
            args.push(full_path.to_string());
        }
        for w in warning_control {
            args.push("-w".to_string());
            args.push(w.to_string());
        }
        args.push("-o".to_string());
        let output_path = self.dir.join(output);
        args.push(output_path.to_string());
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

        // Create the parent directory for the output file if it doesn't exist
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .context("Creating output directory for MoonBit package")?;
        }

        MOONC.run(args)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
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
        let module_wasm_path = self.module_wasm();
        args.push(module_wasm_path.to_string());
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

        // Create the parent directory for the module WASM file if it doesn't exist
        if let Some(parent) = module_wasm_path.parent() {
            std::fs::create_dir_all(parent).context("Creating output directory for module WASM")?;
        }

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

        let module_wasm = self.module_wasm();
        let mut wasm = std::fs::read(&module_wasm)
            .context(format!("Failed to read module WASM from {module_wasm}"))?;

        wit_component::embed_component_metadata(&mut wasm, resolve, *world, StringEncoding::UTF16)
            .context("Embedding component metadata")?;

        let module_with_embed_path = self.module_with_embed_wasm();
        if let Some(parent) = module_with_embed_path.parent() {
            std::fs::create_dir_all(parent)
                .context("Creating output directory for embedded WASM")?;
        }
        std::fs::write(module_with_embed_path, wasm)
            .context("Writing WASM with embedded metadata")?;

        Ok(())
    }

    fn create_component(&self, target: &Utf8Path) -> anyhow::Result<()> {
        info!("Creating the final WASM component at {target}");

        let wasm = std::fs::read(self.module_with_embed_wasm())
            .context("Reading WASM with embedded metadata")?;
        let mut encoder = ComponentEncoder::default()
            .validate(true)
            .reject_legacy_names(false)
            .merge_imports_based_on_semver(true)
            .realloc_via_memory_grow(false)
            .module(&wasm)?;

        let component = encoder.encode().context("Encoding WASM component")?;

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).context("Creating directory for WASM component")?;
        }
        std::fs::write(target, component).context("Writing WASM component")?;

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
                    format!("{root_package}/{dep}")
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
        debug!("Extracting Wasm linker config from {package_json_path}");
        let json_str = std::fs::read_to_string(package_json_path)?;
        let pkg: PackageJsonWithWasmLinkerConfig = serde_json::from_str(&json_str)?;

        Ok(pkg.link.wasm)
    }

    pub fn moonbit_root_package(&self) -> anyhow::Result<String> {
        Ok(format!(
            "{}/{}",
            self.root_pkg_namespace()?,
            self.root_pkg_name()?
        ))
    }

    pub fn root_pkg_namespace(&self) -> anyhow::Result<String> {
        let root_package_id = self.root_package_id.as_ref().unwrap();
        let resolve = self.resolve.as_ref().unwrap();

        let root_package = resolve
            .packages
            .get(*root_package_id)
            .ok_or_else(|| anyhow!("Root package not found"))?;
        Ok(root_package.name.namespace.to_string())
    }

    pub fn root_pkg_name(&self) -> anyhow::Result<String> {
        let root_package_id = self.root_package_id.as_ref().unwrap();
        let resolve = self.resolve.as_ref().unwrap();

        let root_package = resolve
            .packages
            .get(*root_package_id)
            .ok_or_else(|| anyhow!("Root package not found"))?;
        Ok(root_package.name.name.to_string())
    }

    fn world_name(&self) -> anyhow::Result<String> {
        Ok(self
            .resolve
            .as_ref()
            .and_then(|r| r.worlds.get(self.world_id?))
            .map(|w| w.name.to_string())
            .ok_or_else(|| anyhow::anyhow!("Could not find world"))?
            .to_lower_camel_case())
    }

    fn get_imported_interfaces(&self) -> anyhow::Result<Vec<(PackageName, String)>> {
        let world = self
            .resolve
            .as_ref()
            .and_then(|r| r.worlds.get(self.world_id?))
            .ok_or_else(|| anyhow::anyhow!("Could not find world"))?;
        let mut imported_interfaces = Vec::new();
        for (_, item) in &world.imports {
            if let wit_parser::WorldItem::Interface { id, .. } = item
                && let Some(interface) = self.resolve.as_ref().and_then(|r| r.interfaces.get(*id))
            {
                if let Some(interface_name) = interface.name.as_ref() {
                    let owner_package = interface.package.ok_or_else(|| {
                        anyhow::anyhow!("Interface '{}' does not have a package", interface_name)
                    })?;
                    let package = self
                        .resolve
                        .as_ref()
                        .and_then(|r| r.packages.get(owner_package))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Package for interface '{}' not found", interface_name)
                        })?;

                    imported_interfaces.push((package.name.clone(), interface_name.to_string()));
                } else {
                    return Err(anyhow::anyhow!(
                        "Anonymous imported interfaces are not supported"
                    ));
                }
            }
        }
        Ok(imported_interfaces)
    }

    fn get_exported_interfaces(&self) -> anyhow::Result<Vec<(PackageName, String)>> {
        let world = self
            .resolve
            .as_ref()
            .and_then(|r| r.worlds.get(self.world_id?))
            .ok_or_else(|| anyhow::anyhow!("Could not find world"))?;
        let mut exported_interfaces = Vec::new();
        for (_, item) in &world.exports {
            if let wit_parser::WorldItem::Interface { id, .. } = item
                && let Some(interface) = self.resolve.as_ref().and_then(|r| r.interfaces.get(*id))
            {
                if let Some(interface_name) = interface.name.as_ref() {
                    let owner_package = interface.package.ok_or_else(|| {
                        anyhow::anyhow!("Interface '{}' does not have a package", interface_name)
                    })?;
                    let package = self
                        .resolve
                        .as_ref()
                        .and_then(|r| r.packages.get(owner_package))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Package for interface '{}' not found", interface_name)
                        })?;

                    exported_interfaces.push((package.name.clone(), interface_name.to_string()));
                } else {
                    return Err(anyhow::anyhow!(
                        "Anonymous exported interfaces are not supported"
                    ));
                }
            }
        }
        Ok(exported_interfaces)
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

#[cfg(test)]
test_r::enable!();

#[cfg(test)]
struct Trace;

#[cfg(test)]
#[test_r::test_dep]
fn initialize_trace() -> Trace {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Debug)
        .write_style(pretty_env_logger::env_logger::WriteStyle::Always)
        .init();
    Trace
}
