use std::path::{Path, PathBuf};
use std::process::Command;

use ecow::eco_format;
use parking_lot::Mutex;
use std::collections::HashMap;
use typst::diag::{FileError, FileResult, StrResult, Warned};
use typst::foundations::{Bytes, Content, Datetime, Duration};
use typst::syntax::{FileId, Source, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, World};
use typst_kit::diagnostics::DiagnosticWorld;

use crate::args::DiagnosticFormat;
use crate::compile::print_diagnostics;
use crate::world::SystemWorld;

/// Evaluates the document as it was at the given git revision, for use as the
/// base of a diff.
///
/// Diagnostics of the old version are printed directly since their spans
/// refer to the old sources.
pub fn evaluate_base(
    world: &SystemWorld,
    rev: &str,
    format: DiagnosticFormat,
) -> StrResult<Content> {
    let base = GitWorld::new(world, rev)?;
    let Warned { output, .. } = typst::evaluate(&base);
    output.map_err(|errors| {
        print_diagnostics(&base, &errors, &[], format).ok();
        eco_format!("failed to compile the base version at revision `{rev}`")
    })
}

/// A world that provides the project files as they were at a git revision.
///
/// Everything else (packages, fonts, the library) is shared with the
/// underlying system world.
struct GitWorld<'a> {
    /// The world for the current version.
    world: &'a SystemWorld,
    /// The git revision to read project files from.
    rev: String,
    /// The project root, which must be within a git repository.
    root: PathBuf,
    /// Files read from git so far.
    files: Mutex<HashMap<FileId, FileResult<Bytes>>>,
    /// Sources parsed so far.
    sources: Mutex<HashMap<FileId, FileResult<Source>>>,
}

impl<'a> GitWorld<'a> {
    fn new(world: &'a SystemWorld, rev: &str) -> StrResult<Self> {
        let root = world.root().to_path_buf();
        let output = git(&root)
            .args(["rev-parse", "--verify", "--end-of-options"])
            .arg(format!("{rev}^{{commit}}"))
            .output()
            .map_err(|err| eco_format!("failed to run git ({err})"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(eco_format!(
                "failed to resolve git revision `{rev}` ({})",
                stderr.trim().trim_start_matches("fatal: ")
            ));
        }

        Ok(Self {
            world,
            rev: rev.into(),
            root,
            files: Mutex::default(),
            sources: Mutex::default(),
        })
    }

    /// Reads a project file at the revision.
    fn read(&self, id: FileId) -> FileResult<Bytes> {
        self.files
            .lock()
            .entry(id)
            .or_insert_with(|| {
                let vpath = id.vpath().get_without_slash();
                let path = self.root.join(vpath);
                let output = git(&self.root)
                    .arg("show")
                    .arg(format!("{}:./{vpath}", self.rev))
                    .output()
                    .map_err(|err| FileError::from_io(err, &path))?;
                if !output.status.success() {
                    return Err(FileError::NotFound(path));
                }
                Ok(Bytes::new(output.stdout))
            })
            .clone()
    }
}

impl World for GitWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        self.world.library()
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.world.book()
    }

    fn main(&self) -> FileId {
        self.world.main()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if !matches!(id.root(), VirtualRoot::Project) {
            return self.world.source(id);
        }

        if let Some(source) = self.sources.lock().get(&id) {
            return source.clone();
        }

        let source = self.read(id).and_then(|bytes| {
            let text = std::str::from_utf8(&bytes).map_err(|_| FileError::InvalidUtf8)?;
            Ok(Source::new(id, text.into()))
        });
        self.sources.lock().insert(id, source.clone());
        source
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        match id.root() {
            VirtualRoot::Project => self.read(id),
            VirtualRoot::Package(_) => self.world.file(id),
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.world.font(index)
    }

    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        self.world.today(offset)
    }
}

impl DiagnosticWorld for GitWorld<'_> {
    fn name(&self, id: FileId) -> String {
        format!("{} ({})", self.world.name(id), self.rev)
    }
}

/// Creates a git command that runs in the given directory.
fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}
