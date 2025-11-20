use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use std::path::PathBuf;
use std::{env, io::Read};
use tar::Archive;

const ARCHIVE_URL: &str = "https://github.com/moonbitlang/moonbit-compiler/releases/download/v0.6.32%2B6f48aae3f/moonbit-wasm.tar.gz";
const TARGET_DIR_IN_ARCHIVE: &str = "./moonc.assets";
const TARGET_EXTENSION: &str = "wasm";
const FINAL_FILE_NAME: &str = "moonc.wasm";
const MOON_VERSION: &str = "./moon_version";
const CORE_TGZ: &str = "./core.tar.gz";

fn main() -> Result<()> {
    // Tell Cargo to rerun this script if build.rs changes
    println!("cargo:rerun-if-changed=build.rs");

    //  Get the OUT_DIR environment variable set by Cargo
    //  This is the recommended location for our downloaded file
    let out_dir = env::var("OUT_DIR").context("Failed to get OUT_DIR environment variable")?;
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .context("Failed to get CARGO_MANIFEST_DIR environment variable")?;
    let dest_path = PathBuf::from(out_dir.clone()).join(FINAL_FILE_NAME);
    let manifest_path = PathBuf::from(manifest_dir);
    let bundled_core_path = manifest_path.join("bundled-core");

    //  Download the file using reqwest (blocking)
    //  Using a blocking request is simpler in a build script
    println!("cargo:warning=Downloading moonc.wasm...");
    let response = reqwest::blocking::get(ARCHIVE_URL)
        .with_context(|| format!("Failed to download file from URL: {}", ARCHIVE_URL))?;

    // Check the HTTP response status
    if !response.status().is_success() {
        anyhow::bail!(
            "Received an error status code while downloading the file: {}",
            response.status()
        );
    }

    let content = response
        .bytes()
        .context("Failed to read response content")?;

    // Decompress and extract the specific file from the archive in memory
    let tar = GzDecoder::new(content.as_ref());
    let mut archive = Archive::new(tar);

    let mut entry_found = false;
    for entry_result in archive.entries()? {
        let mut entry = entry_result.context("Failed to read archive entry")?;
        let path = entry.path()?.to_path_buf();
        // Check if the entry is in the target directory and has the correct extension
        if !entry_found
            && path.starts_with(TARGET_DIR_IN_ARCHIVE)
            && path.extension().and_then(|s| s.to_str()) == Some(TARGET_EXTENSION)
        {
            println!("cargo:info=Found target file in archive: {:?}", path);

            entry
                .unpack(&dest_path)
                .with_context(|| format!("Failed to unpack file {:?} to {:?}", path, dest_path))?;

            entry_found = true;
        }

        if let Some(str) = path.to_str() {
            if str == MOON_VERSION {
                let mut version = String::new();
                entry.read_to_string(&mut version).with_context(|| {
                    format!("Failed to read version file {:?} in Archive", path)
                })?;
                println!("cargo:warning={:?} = {:?}", str, version.trim());
            }
            if str == CORE_TGZ && !bundled_core_path.exists() {
                println!("cargo:info=Found core.tar.gz in archive, extracting to core",);
                let core_tgz_path = PathBuf::from(out_dir.clone()).join("core.tar.gz");
                entry.unpack(&core_tgz_path).with_context(|| {
                    format!("Failed to unpack file {:?} to {:?}", path, core_tgz_path)
                })?;
                let bytes = std::fs::read(core_tgz_path)
                    .with_context(|| format!("Failed to read file {:?}", path))?;
                let core_tar = GzDecoder::new(bytes.as_slice());
                let mut core_archive = Archive::new(core_tar);

                let core_target = manifest_path.clone();
                let core_dir = manifest_path.join("core");
                if core_dir.exists() {
                    std::fs::remove_dir_all(core_dir).with_context(|| {
                        format!(
                            "Failed to remove directory {:?}",
                            manifest_path.join("core")
                        )
                    })?;
                }
                core_archive.unpack(&core_target).with_context(|| {
                    format!("Failed to unpack file {:?} to {:?}", path, core_target)
                })?;
            }
        }
    }

    if !entry_found {
        anyhow::bail!(
            "Could not find a file with extension '.{}' in the directory '{}' within the downloaded archive.",
            TARGET_EXTENSION,
            TARGET_DIR_IN_ARCHIVE
        );
    }

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=tdh");
    }

    Ok(())
}
