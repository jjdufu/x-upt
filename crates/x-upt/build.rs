//! Build script validating `web/dist` freshness before embedding into `x-upt`.

use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = std::io::stdout();
    writeln!(out, "cargo:rustc-check-cfg=cfg(frontend_embedded)")?;
    writeln!(out, "cargo:rerun-if-env-changed=X_UPT_SKIP_FRONTEND_EMBED")?;
    writeln!(out, "cargo:rerun-if-changed=../../web/dist")?;
    writeln!(out, "cargo:rerun-if-changed=../../web/dist/index.html")?;

    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let dist_dir = manifest_dir.join("../../web/dist");
    let index_html = dist_dir.join("index.html");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let skip = std::env::var("X_UPT_SKIP_FRONTEND_EMBED").as_deref() == Ok("1");

    if skip && profile != "release" {
        return Ok(());
    }

    if !index_html.is_file() {
        return Err(format!(
            "missing frontend bundle at {}; build web/ first",
            index_html.display()
        )
        .into());
    }

    let html = std::fs::read_to_string(&index_html)?;
    if html.trim().is_empty() {
        return Err("web/dist/index.html must not be empty".into());
    }

    writeln!(out, "cargo:rustc-cfg=frontend_embedded")?;
    Ok(())
}
