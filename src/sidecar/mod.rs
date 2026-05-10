mod input;
mod legacy_hunk;
mod review_notes;

pub(crate) use input::{read_legacy_hunk_agent_context_file, read_review_notes_sidecar_file};
pub use legacy_hunk::parse_hunk_agent_context;
pub use review_notes::{
    DiagnosticLevel, OrderedReviewNoteFiles, ParsedReviewNotes, ResolvedReviewNotes,
    ReviewNoteEntry, ReviewNoteTarget, ReviewNotesDiagnostic, ReviewNotesDiagnosticCode,
    ReviewNotesFile, ReviewNotesSidecar, apply_review_notes_file_order as apply_file_order,
    parse_review_notes_sidecar, resolve_notes,
};
