//! Reading Java stdlib classes out of a JDK home, so a `bind_java_type!` binding
//! can be verified against a JDK type (e.g. `java.nio.ByteBuffer`) that is not
//! passed on `--java`.
//!
//! Two on-disk layouts are supported:
//! * **Java 9+**: `$JAVA_HOME/jmods/*.jmod` — a 4-byte `JM` + major/minor version
//!   header followed by a standard ZIP; class entries live under a `classes/`
//!   prefix (`classes/java/nio/ByteBuffer.class`).
//! * **Java 8**: `$JAVA_HOME/{jre/lib,lib}/rt.jar` — a plain JAR.
//!
//! The `lib/modules` jimage runtime image is not supported: a full JDK ships
//! `jmods/`, a JRE does not, and jimage is a bespoke non-ZIP format.

use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::ZipArchive;
use zip::read::{ArchiveOffset, Config as ZipConfig};

/// The 4-byte JMOD header: magic `'J' 'M'` (`0x4A 0x4D`) plus a major/minor
/// version byte. The embedded ZIP begins immediately after it.
const JMOD_MAGIC: [u8; 2] = [0x4A, 0x4D];
const JMOD_HEADER_LEN: u64 = 4;

/// A lazily-searched set of JDK class sources discovered under a JDK home.
pub struct JdkResolver {
    sources: Vec<Source>,
}

struct Source {
    archive: ZipArchive<File>,
    /// Path prefix carried by this source's class entries: `classes/` for a
    /// jmod, empty for a plain jar.
    prefix: &'static str,
}

impl JdkResolver {
    /// Open the class sources under `java_home`: every `jmods/*.jmod` (Java 9+),
    /// else `rt.jar` (Java 8). Returns `None` when the home has neither — e.g. it
    /// is a JRE, or not a JDK at all — so the caller can fall back to W004.
    ///
    /// `java.base.jmod` is searched first: it holds nearly every referenced
    /// stdlib class, so most lookups hit the first source.
    pub fn open(java_home: &Path) -> Option<Self> {
        let mut sources = Vec::new();

        let jmods = java_home.join("jmods");
        if jmods.is_dir() {
            let mut paths: Vec<PathBuf> = std::fs::read_dir(&jmods)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jmod"))
                .collect();
            // java.base first, then the rest in a stable order.
            paths.sort_by_key(|p| {
                (
                    p.file_name() != Some(OsStr::new("java.base.jmod")),
                    p.clone(),
                )
            });
            for p in paths {
                if let Some(archive) = open_jmod(&p) {
                    sources.push(Source {
                        archive,
                        prefix: "classes/",
                    });
                }
            }
        }

        if sources.is_empty() {
            for rt in ["jre/lib/rt.jar", "lib/rt.jar"] {
                if let Ok(file) = File::open(java_home.join(rt))
                    && let Ok(archive) = ZipArchive::new(file)
                {
                    sources.push(Source {
                        archive,
                        prefix: "",
                    });
                    break;
                }
            }
        }

        (!sources.is_empty()).then_some(Self { sources })
    }

    /// The raw bytes of class `internal` (an internal binary name such as
    /// `java/nio/ByteBuffer`), if present in any source.
    pub fn class_bytes(&mut self, internal: &str) -> Option<Vec<u8>> {
        for src in &mut self.sources {
            let name = format!("{}{internal}.class", src.prefix);
            if let Ok(mut entry) = src.archive.by_name(&name) {
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                if entry.read_to_end(&mut bytes).is_ok() {
                    return Some(bytes);
                }
            }
        }
        None
    }
}

/// Open a `.jmod` as a ZIP, validating the `JM` magic and telling the zip reader
/// the archive starts past the 4-byte header. `None` if the file is missing, not
/// a jmod, or not a readable ZIP.
fn open_jmod(path: &Path) -> Option<ZipArchive<File>> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).ok()?;
    if magic != JMOD_MAGIC {
        return None;
    }
    ZipArchive::with_config(
        ZipConfig {
            archive_offset: ArchiveOffset::Known(JMOD_HEADER_LEN),
        },
        file,
    )
    .ok()
}
