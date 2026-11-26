//! Build script compiling `proto/x_upt.proto` into a self-contained `FileDescriptorSet`.

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_dir = manifest_dir.join("../../proto");
    let proto_file = proto_dir.join("x_upt.proto");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor = out.join("x_upt_descriptor.bin");

    let status = std::process::Command::new(protoc)
        .arg("-I")
        .arg(&proto_dir)
        .arg("--include_imports")
        .arg(format!("--descriptor_set_out={}", descriptor.display()))
        .arg(&proto_file)
        .status()?;
    if !status.success() {
        return Err("protoc descriptor compilation failed".into());
    }
    writeln!(
        std::io::stdout(),
        "cargo:rerun-if-changed={}",
        proto_file.display()
    )?;
    Ok(())
}
