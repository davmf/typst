use std::collections::BTreeMap;

use ecow::{EcoVec, eco_format};
use typst::World;
use typst::diag::{HintedStrResult, SourceDiagnostic, StrResult, Warned, bail};
use typst::syntax::{FileId, VirtualRoot};
use typst_docx::{ChangeMode, ImportOptions};
use typst_html::HtmlDocument;

use crate::args::{DocxChanges, DocxImportCommand, Feature, OutputFormat, ProcessArgs};
use crate::compile::print_diagnostics;
use crate::world::SystemWorld;

/// Enables the features DOCX export needs: it compiles through the HTML
/// target and lays out content that HTML can't represent as images.
pub fn process_args(args: &ProcessArgs, format: OutputFormat) -> ProcessArgs {
    let mut args = args.clone();
    if format == OutputFormat::Docx {
        for feature in [Feature::Html, Feature::Docx] {
            if !args.features.contains(&feature) {
                args.features.push(feature);
            }
        }
    }
    args
}

/// Removes the warning about HTML export being experimental, which would be
/// confusing for DOCX export.
pub fn filter_warnings(warnings: EcoVec<SourceDiagnostic>) -> EcoVec<SourceDiagnostic> {
    warnings
        .into_iter()
        .filter(|w| !w.message.starts_with("html export is under active development"))
        .collect()
}

/// Execute a `docx-import` command.
pub fn import(command: &'static DocxImportCommand) -> HintedStrResult<()> {
    let process = process_args(&command.process, OutputFormat::Docx);
    let world = SystemWorld::new(Some(&command.input), &command.world, &process)
        .map_err(|err| eco_format!("{err}"))?;

    let Warned { output, warnings } = typst::compile::<HtmlDocument>(&world);
    let warnings = filter_warnings(warnings);
    let document = match output {
        Ok(document) => document,
        Err(errors) => {
            print_diagnostics(
                &world,
                &errors,
                &warnings,
                command.process.diagnostic_format,
            )
            .map_err(|err| eco_format!("failed to print diagnostics ({err})"))?;
            bail!("failed to compile the document");
        }
    };

    let docx = std::fs::read(&command.docx).map_err(|err| {
        eco_format!("failed to read {} ({err})", command.docx.display())
    })?;
    let options = ImportOptions {
        changes: match command.changes {
            DocxChanges::Apply => ChangeMode::Apply,
            DocxChanges::Annotate => ChangeMode::Annotate,
            DocxChanges::Ignore => ChangeMode::Ignore,
        },
    };
    let report = typst_docx::import_review(&world, &document, &docx, &options)?;

    for item in &report.items {
        let location = item
            .location
            .map(|(id, pos)| format!(" ({})", describe(&world, id, pos)))
            .unwrap_or_default();
        let author = if item.author.is_empty() {
            String::new()
        } else {
            format!(" by {}", item.author)
        };
        println!("{}{author}: {}\n  → {}{location}", item.kind, item.text, item.outcome);
    }
    if report.items.is_empty() {
        println!("no comments or tracked changes found");
    }

    if command.dry_run {
        for edit in &report.edits {
            let source = world.source(edit.id).map_err(|err| eco_format!("{err}"))?;
            let old = &source.text()[edit.range.clone()];
            println!(
                "{}: replace {:?} with {:?}",
                describe(&world, edit.id, edit.range.start),
                old,
                edit.text
            );
        }
        return Ok(());
    }

    let mut by_file: BTreeMap<String, (FileId, Vec<&typst_docx::Edit>)> = BTreeMap::new();
    for edit in &report.edits {
        let key = edit.id.vpath().get_with_slash().to_string();
        by_file.entry(key).or_insert_with(|| (edit.id, vec![])).1.push(edit);
    }
    for (id, edits) in by_file.into_values() {
        if !matches!(id.root(), VirtualRoot::Project) {
            eprintln!("warning: skipping edits to package file {:?}", id.vpath());
            continue;
        }
        let source = world.source(id).map_err(|err| eco_format!("{err}"))?;
        let mut text = source.text().to_string();
        // The edits are sorted by position and don't overlap. Applying them
        // back to front keeps earlier offsets valid and puts edits at the same
        // position in their original order.
        for edit in edits.into_iter().rev() {
            text.replace_range(edit.range.clone(), &edit.text);
        }
        let path = path_of(&world, id)?;
        std::fs::write(&path, text)
            .map_err(|err| eco_format!("failed to write {} ({err})", path.display()))?;
        println!("updated {}", path.display());
    }
    Ok(())
}

fn path_of(world: &SystemWorld, id: FileId) -> StrResult<std::path::PathBuf> {
    id.vpath()
        .realize(world.root())
        .map_err(|err| eco_format!("failed to resolve {:?} ({err:?})", id.vpath()))
}

fn describe(world: &SystemWorld, id: FileId, pos: usize) -> String {
    let line = world
        .source(id)
        .ok()
        .and_then(|s| s.lines().byte_to_line(pos))
        .map(|l| l + 1)
        .unwrap_or(0);
    format!("{}:{line}", id.vpath().get_without_slash())
}
