//! Hooks — lifecycle event capture and context injection.
//!
//! Called by `cogz capture-event` (CLI) and `capture_event` (MCP tool).
//! For `session_start` and `prompt_submit`, generates a context pack
//! for injection into the agent's context window. For `pre_tool_use`
//! and `post_tool_use`, records the event and optionally an observation.
//! For `file_save`, triggers an incremental code reindex. For
//! `session_end`, runs a consolidation dry-run.

pub mod capture;
pub mod handlers;
pub mod lifecycle;

pub use capture::{CaptureError, CaptureResult, run_capture_event};
pub use lifecycle::{LifecycleEvent, handle_lifecycle_event};
