use std::io::Write;

use clap::{Args, Subcommand};

use crate::cli_tracing::TracingArgs;

pub(super) mod assessment;
pub(super) mod capture;
pub(super) mod common;
pub(super) mod history;
pub(super) mod input_request;
pub(super) mod lineage;
pub(super) mod observation;
pub(super) mod unit;
pub(super) mod validation;

#[derive(Debug, Args)]
pub(super) struct ReviewArgs {
    #[command(subcommand)]
    command: ReviewCommand,
}

#[derive(Debug, Subcommand)]
enum ReviewCommand {
    Assessment(assessment::AssessmentArgs),
    Capture(capture::CaptureArgs),
    History(history::HistoryArgs),
    InputRequest(input_request::InputRequestArgs),
    Lineage(lineage::LineageArgs),
    Observation(observation::ObservationArgs),
    Unit(unit::UnitArgs),
    Validation(validation::ValidationArgs),
}

pub(super) fn run(
    args: ReviewArgs,
    tracing: &TracingArgs,
    stdout: &mut dyn Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        ReviewCommand::Assessment(args) => assessment::run(args, stdout),
        ReviewCommand::Capture(args) => capture::run(args, tracing, stdout),
        ReviewCommand::History(args) => history::run(args, stdout),
        ReviewCommand::InputRequest(args) => input_request::run(args, stdout),
        ReviewCommand::Lineage(args) => lineage::run(args, stdout),
        ReviewCommand::Observation(args) => observation::run(args, stdout),
        ReviewCommand::Unit(args) => unit::run(args, stdout),
        ReviewCommand::Validation(args) => validation::run(args, stdout),
    }
}
