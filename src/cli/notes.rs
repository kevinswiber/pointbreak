use std::io::Write;

use clap::{Args, Subcommand};
use shore::session::{ImportNotesOptions, ImportNotesResult, ProjectionDiagnostic, import_notes};

use crate::cli::input::ReviewInputArgs;
use crate::cli::json;

#[derive(Debug, Args)]
pub(super) struct NotesArgs {
    #[command(subcommand)]
    command: NotesCommand,
}

#[derive(Debug, Subcommand)]
enum NotesCommand {
    Apply(NotesApplyArgs),
}

#[derive(Debug, Args)]
struct NotesApplyArgs {
    #[command(flatten)]
    input: ReviewInputArgs,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NotesApplyDocument {
    schema: &'static str,
    version: u32,
    note_count: usize,
    notes_created: usize,
    notes_existing: usize,
    diagnostics: Vec<ProjectionDiagnostic>,
    state_path: String,
}

pub(super) fn run(
    args: NotesArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        NotesCommand::Apply(args) => {
            tracing::debug!(command = "notes.apply", "command_start");
            notes_apply(args, stdout)
        }
    }
}

fn notes_apply(
    args: NotesApplyArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = import_notes(notes_apply_options(&args.input)?)?;
    let document = NotesApplyDocument::from(result);
    json::write_json(stdout, &document, false)
}

fn notes_apply_options(
    args: &ReviewInputArgs,
) -> Result<ImportNotesOptions, Box<dyn std::error::Error>> {
    let mut options = ImportNotesOptions::new(&args.repo);
    match &args.review_notes {
        Some(review_notes) => {
            options = options.with_review_notes(review_notes);
            Ok(options)
        }
        None => Err("exactly one review-notes input is required".into()),
    }
}

impl From<ImportNotesResult> for NotesApplyDocument {
    fn from(result: ImportNotesResult) -> Self {
        Self {
            schema: "shore.notes-apply",
            version: 1,
            note_count: result.note_count,
            notes_created: result.notes_created,
            notes_existing: result.notes_existing,
            diagnostics: result.diagnostics,
            state_path: result.state_path.to_string_lossy().into_owned(),
        }
    }
}
