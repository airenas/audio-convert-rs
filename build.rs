use std::{env, path::PathBuf};

// build.rs
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let v = option_env!("CARGO_APP_VERSION").unwrap_or("dev");
    println!("cargo:rustc-env=CARGO_APP_VERSION={v}");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("audio_convert_descriptor.bin"))
        .compile_protos(&["./protos/audio_convert.proto"], &["./protos"])?;
    Ok(())
}
