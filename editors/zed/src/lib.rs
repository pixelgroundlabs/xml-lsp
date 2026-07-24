//! Zed extension registering the `xml-lsp` language server.
//!
//! Binary resolution, in order: the user's settings
//! (`lsp.xml-lsp.binary.path`), `xml-lsp` on PATH (dev setups), then a
//! cached download of the latest GitHub release for the current
//! platform. `initialization_options` are passed through verbatim.

use std::fs;

use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, Result};

const REPO: &str = "speedata/xml-lsp";

struct XmlLspExtension {
    cached_binary_path: Option<String>,
}

impl XmlLspExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        if let Some(path) = worktree.which("xml-lsp") {
            return Ok(path);
        }
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok_and(|meta| meta.is_file()) {
                return Ok(path.clone());
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = zed::latest_github_release(
            REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (os, arch) = zed::current_platform();
        let arch_name = match arch {
            zed::Architecture::Aarch64 => "aarch64",
            zed::Architecture::X8664 => "x86_64",
            zed::Architecture::X86 => return Err("32-bit x86 is not supported".into()),
        };
        let (os_name, archive_ext, file_type) = match os {
            zed::Os::Mac => ("apple-darwin", "tar.gz", zed::DownloadedFileType::GzipTar),
            zed::Os::Linux => (
                "unknown-linux-gnu",
                "tar.gz",
                zed::DownloadedFileType::GzipTar,
            ),
            zed::Os::Windows => ("pc-windows-msvc", "zip", zed::DownloadedFileType::Zip),
        };
        let asset_name = format!(
            "xml-lsp-{version}-{arch_name}-{os_name}.{archive_ext}",
            version = release.version,
        );
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("release {} has no asset {asset_name}", release.version))?;

        let version_dir = format!("xml-lsp-{}", release.version);
        let binary_path = match os {
            zed::Os::Windows => format!("{version_dir}/xml-lsp.exe"),
            _ => format!("{version_dir}/xml-lsp"),
        };

        if !fs::metadata(&binary_path).is_ok_and(|meta| meta.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );
            zed::download_file(&asset.download_url, &version_dir, file_type)?;
            zed::make_file_executable(&binary_path)?;

            // Drop caches of older versions.
            if let Ok(entries) = fs::read_dir(".") {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with("xml-lsp-") && name != version_dir {
                        let _ = fs::remove_dir_all(entry.path());
                    }
                }
            }
        }

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for XmlLspExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
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
        let command = self.language_server_binary_path(language_server_id, worktree)?;
        Ok(zed::Command {
            command,
            args: Vec::new(),
            env: Vec::new(),
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
