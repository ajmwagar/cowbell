//! Shell out to the right extractor for a sniffed container.

use crate::container::{sniff, Container};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub fn run(package: &Path, out: &Path, dry_run: bool) -> Result<()> {
    let container = sniff(package)?;
    println!("container: {}", container.describe());

    // Roland updater tarballs are handled natively (more robust + testable than
    // shelling out to `tar`, and lets us strip the `./_tmp/` prefix safely).
    if container == Container::Tar {
        return unpack_tar(package, out, dry_run);
    }

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
        Container::Tar => unreachable!("tar is handled natively before plan()"),
        Container::Unknown => anyhow::bail!(
            "unrecognized container; cannot pick an extractor. Try \
             `fw-analyze inspect --scan` or `binwalk -e` to find embedded data."
        ),
    };
    Ok(plan)
}

/// Extract a tar archive natively, stripping the `./_tmp/` (and any leading
/// `./`) prefix so members land directly in `out`. Path-safe: any member with a
/// `..`, absolute, or drive-prefixed path is refused rather than written.
fn unpack_tar(package: &Path, out: &Path, dry_run: bool) -> Result<()> {
    let f = fs::File::open(package).with_context(|| format!("opening {}", package.display()))?;
    let mut archive = tar::Archive::new(f);

    if !dry_run {
        fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    }

    let mut count = 0usize;
    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading tar entry")?;
        let size = entry.header().size()?;
        let raw = entry.path()?.into_owned();

        // Only regular files are extracted; directories are implied by parents,
        // and symlinks/hardlinks are skipped (they could point outside `out`).
        let etype = entry.header().entry_type();
        if etype.is_dir() {
            continue;
        }
        if !etype.is_file() {
            println!("  skip ({etype:?}): {}", raw.display());
            continue;
        }

        let rel = match sanitize_member_path(&raw) {
            Some(rel) if !rel.as_os_str().is_empty() => rel,
            _ => {
                println!("  refuse (unsafe path): {}", raw.display());
                continue;
            }
        };

        let dest = out.join(&rel);
        if dry_run {
            println!("  {size:>10}  {}  ->  {}", raw.display(), rel.display());
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            entry
                .unpack(&dest)
                .with_context(|| format!("extracting {}", dest.display()))?;
            println!("  {size:>10}  {}", rel.display());
        }
        count += 1;
    }

    if dry_run {
        println!("(dry run — nothing written; {count} member(s) listed)");
    } else {
        println!("done — extracted {count} member(s) into {}", out.display());
        println!(
            "next:  `fw-extract carve {}` to locate the firmware payload",
            out.display()
        );
    }
    Ok(())
}

/// Normalize a tar member path for extraction: drop leading `./` components,
/// strip a single leading `_tmp` wrapper (Roland's convention), and reject
/// anything that would escape the output dir. Returns `None` for unsafe paths.
fn sanitize_member_path(raw: &Path) -> Option<PathBuf> {
    let mut comps: Vec<&std::ffi::OsStr> = Vec::new();
    for c in raw.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(s) => comps.push(s),
            // Absolute roots, Windows prefixes, and `..` could escape `out`.
            Component::RootDir | Component::Prefix(_) | Component::ParentDir => return None,
        }
    }
    // Strip a single leading `_tmp/` wrapper if present.
    if comps.first().is_some_and(|s| *s == std::ffi::OsStr::new("_tmp")) {
        comps.remove(0);
    }
    Some(comps.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic tar in memory with one member. SYNTHETIC ONLY.
    fn synthetic_tar(member: &str, data: &[u8]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, member, data)
            .expect("append synthetic member");
        builder.into_inner().expect("finish synthetic tar")
    }

    #[test]
    fn unpack_strips_tmp_prefix() {
        let data = b"not real firmware";
        let tar = synthetic_tar("./_tmp/fake_up.bin", data);

        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("update.bin");
        fs::write(&archive, &tar).expect("write archive");

        let out = dir.path().join("out");
        run(&archive, &out, false).expect("unpack");

        // Prefix stripped: member lands directly in `out`, not under `_tmp/`.
        let extracted = out.join("fake_up.bin");
        assert!(extracted.is_file(), "expected {}", extracted.display());
        assert_eq!(fs::read(&extracted).unwrap(), data);
        assert!(!out.join("_tmp").exists(), "_tmp prefix should be stripped");
    }

    #[test]
    fn sanitize_refuses_parent_traversal() {
        assert_eq!(sanitize_member_path(Path::new("../evil.bin")), None);
        assert_eq!(sanitize_member_path(Path::new("./_tmp/../../evil.bin")), None);
        assert_eq!(sanitize_member_path(Path::new("/etc/passwd")), None);
    }

    #[test]
    fn sanitize_strips_prefixes() {
        assert_eq!(
            sanitize_member_path(Path::new("./_tmp/fake_up.bin")),
            Some(PathBuf::from("fake_up.bin"))
        );
        assert_eq!(
            sanitize_member_path(Path::new("_tmp/sub/x.bin")),
            Some(PathBuf::from("sub/x.bin"))
        );
    }
}
