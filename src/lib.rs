use zed_extension_api::{
    self as zed,
    serde_json,
    settings::LspSettings,
    Architecture, GithubReleaseOptions, LanguageServerId, Os, Result,
};
use std::fs;

const SERVER_BINARY_NAME: &str = "px-to-rem-lsp";
const GITHUB_REPO: &str = "ugi-dev/px-to-rem";
const SERVER_VERSION: &str = "0.1.0";

struct PxToRemExtension {
    cached_binary_path: Option<String>,
}

impl PxToRemExtension {
    fn server_binary_path(&mut self, language_server_id: &LanguageServerId) -> Result<String> {
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok() {
                return Ok(path.clone());
            }
        }

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );

        let (platform, arch) = zed::current_platform();

        let os_str = match platform {
            Os::Mac => "darwin",
            Os::Linux => "linux",
            Os::Windows => "windows",
        };
        let arch_str = match arch {
            Architecture::Aarch64 => "aarch64",
            Architecture::X8664 => "x86_64",
            _ => return Err("Unsupported CPU architecture".into()),
        };
        let ext = if platform == Os::Windows { ".exe" } else { "" };

        let asset_name = format!(
            "{SERVER_BINARY_NAME}-{SERVER_VERSION}-{os_str}-{arch_str}{ext}"
        );

        let release = zed::latest_github_release(
            GITHUB_REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("No release asset found for your platform: {asset_name}"))?;

        let binary_path = format!("bin/{SERVER_BINARY_NAME}");

        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::Downloading,
        );

        zed::download_file(
            &asset.download_url,
            &binary_path,
            zed::DownloadedFileType::Uncompressed,
        )?;

        zed::make_file_executable(&binary_path)?;

        self.cached_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }
}

impl zed::Extension for PxToRemExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // Prefer a binary already on PATH (useful for local development)
        if let Some(path) = worktree.which(SERVER_BINARY_NAME) {
            return Ok(zed::Command {
                command: path,
                args: vec![],
                env: Default::default(),
            });
        }

        let binary_path = self.server_binary_path(language_server_id)?;

        Ok(zed::Command {
            command: binary_path,
            args: vec![],
            env: Default::default(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<serde_json::Value>> {
        // Read user-configured LSP settings and forward as initialization options.
        // Users set these in ~/.config/zed/settings.json under:
        //   "lsp": { "px-to-rem-lsp": { "initialization_options": { ... } } }
        let user_settings = LspSettings::for_worktree(server_id.as_ref(), worktree)
            .ok()
            .and_then(|s| s.initialization_options);

        // Fall back to sensible defaults matching the original VSCode extension
        Ok(Some(user_settings.unwrap_or_else(|| {
            serde_json::json!({
                "px_per_rem": 16,
                "decimal_places": 4
            })
        })))
    }
}

zed::register_extension!(PxToRemExtension);
