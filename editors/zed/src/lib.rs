//! Zed extension registering the `xml-lsp` language server.
//!
//! The binary location comes from the user's settings
//! (`lsp.xml-lsp.binary.path`), falling back to `xml-lsp` on PATH.
//! `initialization_options` are passed through verbatim.

use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, Result};

struct XmlLspExtension;

impl zed::Extension for XmlLspExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree("xml-lsp", worktree)?;
        if let Some(binary) = settings.binary {
            if let Some(path) = binary.path {
                return Ok(zed::Command {
                    command: path,
                    args: binary.arguments.unwrap_or_default(),
                    env: Vec::new(),
                });
            }
        }
        worktree
            .which("xml-lsp")
            .map(|path| zed::Command {
                command: path,
                args: Vec::new(),
                env: Vec::new(),
            })
            .ok_or_else(|| {
                format!(
                    "{language_server_id}: no binary found — set lsp.xml-lsp.binary.path \
                     in your settings or put xml-lsp on PATH"
                )
            })
    }

    fn language_server_initialization_options(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<zed::serde_json::Value>> {
        let settings = LspSettings::for_worktree("xml-lsp", worktree)?;
        Ok(settings.initialization_options)
    }
}

zed::register_extension!(XmlLspExtension);
