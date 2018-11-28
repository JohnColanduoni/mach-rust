use std::{env};
use std::path::PathBuf;
use std::io::{Write, BufWriter};
use std::fs::File;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = env::var("TARGET").unwrap();
    let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    if target.contains("apple") {
        // Generate header from requested features
        let mach_header_path = out_path.join("headers.h");

        {
            let header_out = File::create(&mach_header_path).expect("failed to open header file output");
            let mut header_out = BufWriter::new(header_out);

            // Always include some headers
            for header_name in ["mach_types", "boolean", "kern_return", "error", "mach_error"].iter() {
                writeln!(header_out, "#include <mach/{}.h>", header_name).unwrap();
            }

            for (feature_env, _) in env::vars_os() {
                const PREFIX: &str = "CARGO_FEATURE_";
                let feature_env = if let Some(feature_env) = feature_env.to_str() { feature_env } else {
                    continue
                };
                if !feature_env.starts_with(PREFIX) {
                    continue;
                }
                let feature_name = feature_env[PREFIX.len()..].to_ascii_lowercase();
                if feature_name == "default" {
                    continue;
                }
                writeln!(header_out, "#include <mach/{}.h>", feature_name).unwrap();
            }
        }


        let mut bindings = bindgen::Builder::default()
            .header(mach_header_path.to_str().unwrap())
            .derive_debug(false);
        if env::var_os("DEBUG").is_some() {
            bindings = bindings.rustfmt_bindings(true);
        }
        let bindings = bindings
            .generate()
            .expect("failed to generate bindings");

        bindings
            .write_to_file(out_path.join("mach.rs"))
            .expect("failed to write bindings");
    }
}