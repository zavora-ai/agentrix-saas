//! The open-source `mcp-erp` server (its Zavora ERA backend) as one company, reached with
//! adk-rust's MCP client: `adk_tool::mcp::McpToolset` over a child process speaking stdio.
//!
//! mcp-erp logs in to ERA with the company's own login (`ZAVORA_EMAIL`/`ZAVORA_PASSWORD`), so
//! every write it makes is the company's. Its tools answer JSON text, or `Error: …` text that
//! carries ERA's status (`Zavora API 409 Conflict on payments: {...}`), mapped here onto
//! [`BooksError`].

use std::path::PathBuf;

use adk_tool::mcp::McpToolset;
use adk_tool::mcp::rmcp::ServiceExt;
use adk_tool::mcp::rmcp::transport::TokioChildProcess;
use saas_core::BooksError;
use serde_json::Value;

/// Where mcp-erp is and how to start it.
#[derive(Debug, Clone)]
pub struct McpConfig {
    /// The `mcp-erp` binary, built with `--features zavora`.
    pub command: PathBuf,
    /// Its working directory: where its `mcp-server.toml` manifest is (the mcp-erp checkout).
    pub dir: PathBuf,
}

impl McpConfig {
    /// `MCP_ERP_BIN` and `MCP_ERP_DIR`, defaulting to this entry's build cache and the
    /// checkout in `~/Developer/projects/mcp-servers/mcp-erp`. `None` when the binary is missing.
    pub fn from_env() -> Option<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let command = std::env::var_os("MCP_ERP_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                home.join("Developer/hackathons/.agentrix-saas-target/mcp-erp/debug/mcp-erp")
            });
        let dir = std::env::var_os("MCP_ERP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("Developer/projects/mcp-servers/mcp-erp"));
        (command.is_file() && dir.join("mcp-server.toml").is_file())
            .then_some(Self { command, dir })
    }
}

pub(crate) struct ErpMcp {
    toolset: McpToolset,
}

impl ErpMcp {
    /// Starts mcp-erp for one company.
    pub async fn spawn(
        cfg: &McpConfig,
        era_url: &str,
        email: &str,
        password: &str,
    ) -> Result<Self, BooksError> {
        let mut cmd = tokio::process::Command::new(&cfg.command);
        cmd.current_dir(&cfg.dir)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("ZAVORA_API_URL", era_url)
            .env("ZAVORA_EMAIL", email)
            .env("ZAVORA_PASSWORD", password)
            .env("RUST_LOG", "warn");
        let (transport, _) = TokioChildProcess::builder(cmd)
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| BooksError::Unavailable(format!("mcp-erp: {e}")))?;
        let client = ()
            .serve(transport)
            .await
            .map_err(|e| BooksError::Unavailable(format!("mcp-erp: {e}")))?;
        Ok(Self {
            toolset: McpToolset::new(client).with_name("mcp-erp"),
        })
    }

    /// Calls one mcp-erp tool and reads its JSON answer.
    pub async fn call(&self, tool: &str, args: Value) -> Result<Value, BooksError> {
        let args = match args {
            Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        let out = self
            .toolset
            .call_tool_value(tool, args)
            .await
            .map_err(|e| BooksError::Unavailable(format!("mcp-erp {tool}: {e}")))?;
        let text = match &out["output"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => return Ok(other.clone()),
        };
        parse_tool_text(tool, &text)
    }
}

/// mcp-erp's answer text: JSON, or `Error: <message>`.
pub(crate) fn parse_tool_text(tool: &str, text: &str) -> Result<Value, BooksError> {
    match text.strip_prefix("Error: ") {
        Some(err) => Err(map_tool_error(tool, err)),
        None => Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))),
    }
}

/// `Zavora API 409 Conflict on payments: {"error":"..."}` onto a books error.
pub(crate) fn map_tool_error(tool: &str, err: &str) -> BooksError {
    let status = err
        .strip_prefix("Zavora API ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|code| code.parse::<u16>().ok())
        .and_then(|code| reqwest::StatusCode::from_u16(code).ok());
    let detail = err.split_once(": ").map(|(_, d)| d).unwrap_or(err);
    let body: Value =
        serde_json::from_str(detail).unwrap_or_else(|_| Value::String(detail.to_owned()));
    match status {
        Some(s) => crate::rest::map_status(s, &format!("mcp-erp {tool}"), &body),
        None if err.contains("login failed") => {
            BooksError::Backend(format!("mcp-erp {tool}: {err}"))
        }
        None => BooksError::Unavailable(format!("mcp-erp {tool}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_answers_are_json_or_mapped_errors() {
        assert_eq!(
            parse_tool_text("record_payment", r#"{"id": "pay1"}"#).unwrap()["id"],
            "pay1"
        );
        let conflict = parse_tool_text(
            "post_invoice",
            r#"Error: Zavora API 409 Conflict on invoices/inv1/post: {"error":"credit limit exceeded"}"#,
        )
        .unwrap_err();
        assert!(
            matches!(conflict, BooksError::Conflict(ref m) if m.contains("credit limit exceeded")),
            "{conflict:?}"
        );
        let invalid = parse_tool_text(
            "post_journal_entry",
            r#"Error: Zavora API 400 Bad Request on journal-entries: {"error":"Reference 'x' already exists"}"#,
        )
        .unwrap_err();
        assert!(matches!(invalid, BooksError::Invalid(ref m) if m.contains("already exists")));
        let down =
            parse_tool_text("list_payments", "Error: error sending request for url").unwrap_err();
        assert!(down.retryable());
        assert!(matches!(
            parse_tool_text("x", "Error: Zavora login failed (401 Unauthorized)"),
            Err(BooksError::Backend(_))
        ));
    }
}
