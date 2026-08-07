//! Shell out to the right extractor for a sniffed container.

use crate::container::{sniff, Container};
use anyhow::Result;
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn run(package: &Path, out: &Path, dry_run: bool) -> Result<()> {
    let container = sniff(package)?;
    println!("container: {}", container.describe());

    let (tool, args) = plan(container, package, out)?;

    let printable = std::iter::once(tool.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    println!("command:   {printable}");

    if dry_run {
        println!("(dry run — nothing executed)");
        return Ok(());
    }

    fs::create_dir_all(out)?;

    match Command::new(tool).args(&args).status() {
        Ok(status) if status.success() => {
            println!("done — extracted into {}", out.display());
            println!(
                "next:  `fw-extract carve {}` to locate the firmware payload",
                out.display()
            );
            Ok(())
        }
        Ok(status) => anyhow::bail!("{tool} exited with {status}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => anyhow::bail!(
            "`{tool}` not found on PATH. Install it and retry, or run the \
             extraction manually and then use `fw-extract carve`."
        ),
        Err(e) => Err(e.into()),
    }
}

/// Decide the tool + argument vector for a container. Kept pure so it is easy
/// to eyeball via `--dry-run` before actually invoking anything.
fn plan(container: Container, package: &Path, out: &Path) -> Result<(&'static str, Vec<String>)> {
    let pkg = package.display().to_string();
    let out_s = out.display().to_string();

    let plan = match container {
        // 7z handles PE SFX installers, plain zips, 7z archives, and most dmgs.
        Container::PeExe | Container::Zip | Container::SevenZip | Container::Dmg => (
            "7z",
            vec!["x".into(), format!("-o{out_s}"), "-y".into(), pkg],
        ),
        Container::Msi => ("msiextract", vec!["--directory".into(), out_s, pkg]),
        Container::Pkg => ("xar", vec!["-xf".into(), pkg, "-C".into(), out_s]),
        Container::Unknown => anyhow::bail!(
            "unrecognized container; cannot pick an extractor. Try \
             `fw-analyze inspect --scan` or `binwalk -e` to find embedded data."
        ),
    };
    Ok(plan)
}
