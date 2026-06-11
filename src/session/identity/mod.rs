mod clock;
mod writer;

pub(crate) use clock::current_timestamp;
pub(crate) use writer::{is_valid_actor_id, writer_from_git_config, writer_from_options};
