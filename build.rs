fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor = out.join("upupt_descriptor.bin");
    let status = std::process::Command::new(protoc)
        .arg("-I")
        .arg("proto")
        .arg("--include_imports")
        .arg(format!("--descriptor_set_out={}", descriptor.display()))
        .arg("proto/upupt.proto")
        .status()?;
    if !status.success() {
        return Err("protoc failed".into());
    }
    let bytes = std::fs::read(&descriptor)?;
    let fds = prost::Message::decode(bytes.as_slice())?;
    tonic_build::configure()
        .file_descriptor_set_path(&descriptor)
        .compile_fds(fds)?;
    println!("cargo:rerun-if-changed=proto/upupt.proto");
    Ok(())
}
