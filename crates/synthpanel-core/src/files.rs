//! File manager.
//!
//! Every path the panel touches is resolved inside a sandbox root, so a
//! crafted request can never escape into the rest of the filesystem.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// A file or directory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub size: u64,
    pub modified: Option<String>,
    pub mode: String,
    pub owner: String,
    pub group: String,
}

/// A listing of a directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<Entry>,
}

/// The file names the file manager must never hand out.
///
/// The IPC token and the secret key authenticate every request to the
/// privileged core, so reading them through the file API would be a
/// complete bypass of the panel's authorization. The rule is enforced by
/// name, so it holds no matter which scope the file lives in.
const SENSITIVE_NAMES: &[&str] = &[
    "ipc-token",
    "ipc_token",
    "secret.key",
    "secret_key",
    "authorized_keys",
    "shadow",
    ".env",
    // The default names of the OpenSSH private keys.
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
];

/// The file extensions the file manager must never hand out.
const SENSITIVE_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "jks", "keystore"];

/// Whether a file name is one the file manager refuses to touch.
pub fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if SENSITIVE_NAMES.contains(&lower.as_str()) {
        return true;
    }
    let extension = lower.rsplit('.').next().unwrap_or_default();
    SENSITIVE_EXTENSIONS.contains(&extension)
}

/// A sandbox the panel operates in.
#[derive(Debug, Clone)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a user supplied path inside the sandbox.
    ///
    /// The path is treated as relative to the sandbox root unless it is
    /// already absolute *and* inside the root. Every component is checked,
    /// so `..` can never escape.
    pub fn resolve(&self, path: &str) -> Result<PathBuf> {
        let candidate = Path::new(path.trim());
        let relative = if candidate.is_absolute() {
            candidate
                .strip_prefix(&self.root)
                .map_err(|_| anyhow::anyhow!("the path is outside the allowed area"))?
                .to_path_buf()
        } else {
            candidate.to_path_buf()
        };

        let mut resolved = self.root.clone();
        for component in relative.components() {
            match component {
                Component::Normal(part) => resolved.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    bail!("the path may not leave the allowed area")
                }
                Component::RootDir | Component::Prefix(_) => {
                    bail!("the path may not be absolute")
                }
            }
        }

        // Belt and braces: the resolved path must still be inside the root.
        if !resolved.starts_with(&self.root) {
            bail!("the path is outside the allowed area");
        }

        // The secrets of the panel are never readable through the file
        // manager, whatever the scope is.
        if let Some(name) = resolved.file_name().and_then(|name| name.to_str()) {
            if is_sensitive_name(name) {
                bail!("the file does not exist");
            }
        }
        Ok(resolved)
    }

    /// The path relative to the sandbox root, for display.
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .trim_start_matches('/')
            .to_string()
    }

    /// List a directory.
    pub fn list(&self, path: &str) -> Result<Listing> {
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            bail!("the path does not exist");
        }
        if !resolved.is_dir() {
            bail!("the path is not a directory");
        }
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&resolved)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if is_sensitive_name(&name) {
                continue;
            }
            let child = entry.path();
            entries.push(Entry {
                path: self.relative(&child),
                name,
                is_directory: metadata.is_dir(),
                size: if metadata.is_dir() { 0 } else { metadata.len() },
                modified: metadata
                    .modified()
                    .ok()
                    .map(|value| chrono::DateTime::<chrono::Utc>::from(value).to_rfc3339()),
                mode: format_mode(&metadata),
                owner: owner_name(&metadata),
                group: group_name(&metadata),
            });
        }
        entries.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        let parent = resolved
            .parent()
            .filter(|parent| parent.starts_with(&self.root) && parent != &self.root)
            .map(|parent| self.relative(parent));

        Ok(Listing {
            path: self.relative(&resolved),
            parent,
            entries,
        })
    }

    /// Create a directory.
    pub fn create_dir(&self, path: &str) -> Result<()> {
        let resolved = self.resolve(path)?;
        std::fs::create_dir_all(&resolved)?;
        Ok(())
    }

    /// Write a file, creating the parent directories when needed.
    pub fn write_file(&self, path: &str, contents: &[u8]) -> Result<()> {
        let resolved = self.resolve(path)?;
        if let Some(parent) = resolved.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&resolved, contents)?;
        Ok(())
    }

    /// Read a file.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            bail!("the file does not exist");
        }
        Ok(std::fs::read(&resolved)?)
    }

    /// Rename or move an entry.
    pub fn rename(&self, from: &str, to: &str) -> Result<()> {
        let source = self.resolve(from)?;
        let destination = self.resolve(to)?;
        if !source.exists() {
            bail!("the source does not exist");
        }
        if destination.exists() {
            bail!("the destination already exists");
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&source, &destination)?;
        Ok(())
    }

    /// Copy a file or a directory tree.
    ///
    /// Both paths go through [`Self::resolve`], so traversal and sensitive
    /// names are rejected on the source *and* on the destination. Existing
    /// destinations are never overwritten: callers must pick a free name.
    pub fn copy(&self, from: &str, to: &str) -> Result<()> {
        let source = self.resolve(from)?;
        let destination = self.resolve(to)?;
        if !source.exists() {
            bail!("the source does not exist");
        }
        // `exists()` follows symlinks and reports false for a dangling one;
        // writing through such a link would escape the sandbox.
        if std::fs::symlink_metadata(&destination).is_ok() {
            bail!("the destination already exists");
        }
        if source.is_dir() && destination.starts_with(&source) {
            bail!("a directory cannot be copied into itself");
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Err(error) = copy_tree(&source, &destination) {
            // Do not leave half a copy behind when a nested entry fails.
            if destination.is_dir() {
                let _ = std::fs::remove_dir_all(&destination);
            } else {
                let _ = std::fs::remove_file(&destination);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Delete a file or (recursively) a directory.
    pub fn delete(&self, path: &str) -> Result<bool> {
        let resolved = self.resolve(path)?;
        if resolved == self.root {
            bail!("the sandbox root cannot be deleted");
        }
        if !resolved.exists() {
            return Ok(false);
        }
        if resolved.is_dir() {
            std::fs::remove_dir_all(&resolved)?;
        } else {
            std::fs::remove_file(&resolved)?;
        }
        Ok(true)
    }

    /// Change the permissions of an entry.
    pub fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            bail!("the path does not exist");
        }
        if mode > 0o7777 {
            bail!("the mode must be an octal value below 07777");
        }
        std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(mode))?;
        Ok(())
    }

    /// Change the owner and group of an entry.
    pub fn chown(&self, path: &str, owner: Option<&str>, group: Option<&str>) -> Result<()> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            bail!("the path does not exist");
        }
        let path_c = CString::new(resolved.as_os_str().as_bytes())?;
        unsafe {
            let uid: libc::uid_t = match owner {
                Some(name) => {
                    let name_c = CString::new(name)?;
                    let pw = libc::getpwnam(name_c.as_ptr());
                    if pw.is_null() {
                        bail!("unknown user `{name}`");
                    }
                    (*pw).pw_uid
                }
                None => libc::uid_t::MAX,
            };
            let gid: libc::gid_t = match group {
                Some(name) => {
                    let name_c = CString::new(name)?;
                    let gr = libc::getgrnam(name_c.as_ptr());
                    if gr.is_null() {
                        bail!("unknown group `{name}`");
                    }
                    (*gr).gr_gid
                }
                None => libc::gid_t::MAX,
            };
            if libc::chown(path_c.as_ptr(), uid, gid) != 0 {
                bail!("cannot change the owner of the path");
            }
        }
        Ok(())
    }

    /// Create an archive from a directory.
    pub fn archive(&self, path: &str, format: ArchiveFormat) -> Result<(String, u64)> {
        let resolved = self.resolve(path)?;
        if !resolved.exists() {
            bail!("the path does not exist");
        }
        let name = resolved
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".into());
        let archive_name = match format {
            ArchiveFormat::Zip => format!("{name}.zip"),
            ArchiveFormat::TarGz => format!("{name}.tar.gz"),
        };
        let archive_path = resolved.parent().unwrap_or(&self.root).join(&archive_name);
        let size = match format {
            ArchiveFormat::Zip => create_zip(&resolved, &archive_path)?,
            ArchiveFormat::TarGz => create_tar_gz(&resolved, &archive_path)?,
        };
        Ok((self.relative(&archive_path), size))
    }

    /// Extract an archive into a directory.
    pub fn extract(&self, path: &str, into: &str) -> Result<Vec<String>> {
        let resolved = self.resolve(path)?;
        let destination = self.resolve(into)?;
        if !resolved.exists() {
            bail!("the archive does not exist");
        }
        std::fs::create_dir_all(&destination)?;
        let name = resolved.to_string_lossy().to_string();
        if name.ends_with(".zip") {
            extract_zip(&resolved, &destination)?;
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            extract_tar_gz(&resolved, &destination)?;
        } else {
            bail!("only .zip and .tar.gz archives can be extracted");
        }
        Ok(self
            .list(&self.relative(&destination))?
            .entries
            .iter()
            .map(|e| e.path.clone())
            .collect())
    }
}

/// The archive formats the panel can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

/// Copy one entry, recursing into directories.
///
/// Symlinks are refused: following one could make the panel copy data from
/// outside the sandbox. Permissions are carried over from the source.
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!("symbolic links cannot be copied");
    }
    if metadata.is_dir() {
        std::fs::create_dir(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(source, destination)?;
    }
    std::fs::set_permissions(destination, metadata.permissions())?;
    Ok(())
}

fn format_mode(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:04o}", metadata.permissions().mode() & 0o7777)
}

fn owner_name(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    let uid = metadata.uid();
    // Resolve the name from /etc/passwd when possible.
    if let Ok(passwd) = std::fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() > 2 && fields[2].parse::<u32>().ok() == Some(uid) {
                return fields[0].to_string();
            }
        }
    }
    uid.to_string()
}

fn group_name(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    let gid = metadata.gid();
    if let Ok(group) = std::fs::read_to_string("/etc/group") {
        for line in group.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() > 2 && fields[2].parse::<u32>().ok() == Some(gid) {
                return fields[0].to_string();
            }
        }
    }
    gid.to_string()
}

fn create_tar_gz(source: &Path, archive_path: &Path) -> Result<u64> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let file = std::fs::File::create(archive_path)?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    let name = source.file_name().unwrap_or_default();
    if source.is_dir() {
        // `append_dir_all` takes the archive path first, then the source.
        builder.append_dir_all(name, source)?;
    } else {
        builder.append_path_with_name(source, name)?;
    }
    let encoder = builder.into_inner()?;
    let mut file = encoder.finish()?;
    use std::io::Write;
    file.flush()?;
    Ok(std::fs::metadata(archive_path)?.len())
}

fn extract_tar_gz(archive: &Path, destination: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(destination)?;
    Ok(())
}

fn create_zip(source: &Path, archive_path: &Path) -> Result<u64> {
    // The panel produces zip archives with the `zip` CLI; it is part of every
    // supported distribution and avoids another dependency.
    let output = std::process::Command::new("zip")
        .arg("-r")
        .arg("-q")
        .arg(archive_path)
        .arg(source.file_name().unwrap_or_default())
        .current_dir(source.parent().unwrap_or_else(|| Path::new("/")))
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(std::fs::metadata(archive_path)?.len()),
        Ok(output) => bail!(
            "zip failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => bail!("cannot run zip: {error}"),
    }
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let output = std::process::Command::new("unzip")
        .arg("-o")
        .arg("-q")
        .arg(archive)
        .arg("-d")
        .arg(destination)
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => bail!(
            "unzip failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => bail!("cannot run unzip: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> (Sandbox, PathBuf) {
        let root = std::env::temp_dir().join(format!("sp-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        (Sandbox::new(root.clone()), root)
    }

    #[test]
    fn refuses_the_secrets_of_the_panel() {
        assert!(is_sensitive_name("ipc-token"));
        assert!(is_sensitive_name("secret.key"));
        assert!(is_sensitive_name("panel-cert.pem"));
        assert!(is_sensitive_name("id_rsa"));
        assert!(is_sensitive_name("authorized_keys"));
        assert!(!is_sensitive_name("index.html"));
        assert!(!is_sensitive_name("vhost.conf"));
    }

    #[test]
    fn rejects_traversal() {
        let (box_, _) = sandbox();
        assert!(box_.resolve("../etc/passwd").is_err());
        assert!(box_.resolve("a/../../etc/passwd").is_err());
        assert!(box_.resolve("/etc/passwd").is_err());
        assert!(box_.resolve("a/b/../c").is_err());
    }

    #[test]
    fn resolves_inside_the_sandbox() {
        let (box_, root) = sandbox();
        assert_eq!(box_.resolve("a/b.txt").unwrap(), root.join("a/b.txt"));
        assert_eq!(box_.resolve("./a").unwrap(), root.join("a"));
        assert_eq!(box_.resolve("").unwrap(), root);
    }

    #[test]
    fn lists_and_creates() {
        let (box_, root) = sandbox();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.txt"), b"hello").unwrap();

        let listing = box_.list("sub").unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "a.txt");
        assert_eq!(listing.entries[0].size, 5);
        assert!(!listing.entries[0].is_directory);

        box_.create_dir("new/dir").unwrap();
        assert!(root.join("new/dir").is_dir());
    }

    #[test]
    fn writes_reads_and_deletes() {
        let (box_, _root) = sandbox();
        box_.write_file("notes/readme.md", b"# hi").unwrap();
        assert_eq!(box_.read_file("notes/readme.md").unwrap(), b"# hi");
        assert!(box_.delete("notes/readme.md").unwrap());
        assert!(!box_.delete("notes/readme.md").unwrap());
    }

    #[test]
    fn renames_files() {
        let (box_, root) = sandbox();
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        box_.rename("a.txt", "b.txt").unwrap();
        assert!(root.join("b.txt").exists());
        assert!(!root.join("a.txt").exists());
    }

    #[test]
    fn copies_files_and_directories() {
        let (box_, root) = sandbox();
        std::fs::create_dir_all(root.join("data/sub")).unwrap();
        std::fs::write(root.join("data/a.txt"), b"hello").unwrap();
        std::fs::write(root.join("data/sub/b.txt"), b"world").unwrap();

        box_.copy("data", "backup/data").unwrap();
        assert_eq!(box_.read_file("backup/data/a.txt").unwrap(), b"hello");
        assert_eq!(box_.read_file("backup/data/sub/b.txt").unwrap(), b"world");
        assert!(root.join("backup/data").is_dir());

        box_.copy("backup/data/a.txt", "copy.txt").unwrap();
        assert_eq!(box_.read_file("copy.txt").unwrap(), b"hello");
    }

    #[test]
    fn copy_refuses_overwrite_and_missing_source() {
        let (box_, root) = sandbox();
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        std::fs::write(root.join("b.txt"), b"y").unwrap();
        assert!(box_.copy("a.txt", "b.txt").is_err());
        assert_eq!(box_.read_file("b.txt").unwrap(), b"y");
        assert!(box_.copy("missing.txt", "c.txt").is_err());
        assert!(!root.join("c.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn copy_refuses_a_dangling_symlink_destination() {
        let (box_, root) = sandbox();
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(root.join("outside.txt"), root.join("link.txt")).unwrap();
        assert!(box_.copy("a.txt", "link.txt").is_err());
        assert!(!root.join("outside.txt").exists());
    }

    #[test]
    fn copy_applies_sandbox_rules_to_both_paths() {
        let (box_, root) = sandbox();
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        assert!(box_.copy("../etc/passwd", "out.txt").is_err());
        assert!(box_.copy("a.txt", "../out.txt").is_err());
        assert!(box_.copy("a.txt", "secret.key").is_err());
        assert!(box_.copy("ipc-token", "out.txt").is_err());
        assert!(!root.join("out.txt").exists());
    }

    #[test]
    fn copy_refuses_a_recursive_directory_target() {
        let (box_, root) = sandbox();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data/a.txt"), b"x").unwrap();
        assert!(box_.copy("data", "data/nested").is_err());
        assert!(!root.join("data/nested").exists());
    }

    #[test]
    fn cannot_delete_the_root() {
        let (box_, _) = sandbox();
        assert!(box_.delete("").is_err());
    }

    #[test]
    fn changes_permissions() {
        let (box_, root) = sandbox();
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        box_.chmod("a.txt", 0o600).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join("a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn creates_and_extracts_a_tar_archive() {
        let (box_, root) = sandbox();
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::write(root.join("data/file.txt"), b"payload").unwrap();

        let (name, size) = box_.archive("data", ArchiveFormat::TarGz).unwrap();
        assert!(name.ends_with(".tar.gz"));
        assert!(size > 0);

        let extracted = box_.extract(&name, "restored").unwrap();
        assert!(!extracted.is_empty(), "extracted: {extracted:?}");
        assert_eq!(
            box_.read_file("restored/data/file.txt").unwrap(),
            b"payload"
        );
    }
}
