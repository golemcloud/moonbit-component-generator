fn main() {
    #[cfg(target_os = "windows")]
    {
        // Link Windows system libraries required by V8
        println!("cargo:rustc-link-lib=advapi32"); // For registry functions
        println!("cargo:rustc-link-lib=tdh"); // For ETW (Event Tracing for Windows)
        println!("cargo:rustc-link-lib=user32"); // For additional Windows APIs
        println!("cargo:rustc-link-lib=kernel32"); // For kernel functions
    }
}
