//! MCP error helpers — structured error responses for tool handlers.

use rmcp::{ErrorData as McpError, model::ErrorCode};
use serde_json::json;

/// Build an MCP error with a custom error code in structured data.
pub fn mcp_error(code: &str, message: &str) -> McpError {
    McpError::new(
        ErrorCode::INVALID_PARAMS,
        message.to_string(),
        Some(json!({ "code": code })),
    )
}

/// Build an MCP internal error for infrastructure failures.
pub fn mcp_internal_error(context: &str, message: &str) -> McpError {
    McpError::new(
        ErrorCode::INTERNAL_ERROR,
        format!("{}: {}", context, message),
        None,
    )
}

/// Build an MCP invalid-parameter error.
pub fn mcp_invalid_parameter(message: &str) -> McpError {
    McpError::new(ErrorCode::INVALID_PARAMS, message.to_string(), None)
}
