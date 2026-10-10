//! The version check against GitHub Releases.

use eframe::egui;

use crate::status_bar::StatusMessage;
use crate::style::Palette;
use crate::{AppEvent, SoosApp};

#[derive(serde::Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) html_url: String,
}

/// `https://api.github.com/repos/<owner>/<repo>/releases/latest`, from the
/// crate's `repository` field. That field's owner must be current: GitHub's
/// API doesn't follow an account rename.
pub(crate) fn latest_release_url() -> Option<String> {
    let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let (_, path) = repo.split_once("github.com/")?;
    Some(format!(
        "https://api.github.com/repos/{path}/releases/latest"
    ))
}

/// `"v1.2.3"` as numbers, so 0.9.0 sorts before 0.10.0.
pub(crate) fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.strip_prefix('v').unwrap_or(v);
    let mut parts = v.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Runs on a background thread. Any failure is `Failed`, never "up to date".
pub(crate) fn check_latest_release() -> UpdateStatus {
    // GitHub's API rejects requests without a User-Agent.
    let Some((latest, release)) = latest_release_url()
        .and_then(|url| {
            ureq::get(&url)
                .header("User-Agent", "soos-app")
                .config()
                .timeout_global(Some(std::time::Duration::from_secs(10)))
                .build()
                .call()
                .ok()
        })
        .and_then(|mut r| {
            r.body_mut()
                .with_config()
                .limit(soos_core::currency::MAX_RESPONSE_BYTES)
                .read_json::<GithubRelease>()
                .ok()
        })
        .and_then(|release| Some((parse_version(&release.tag_name)?, release)))
    else {
        return UpdateStatus::Failed;
    };
    let current = parse_version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0));
    if latest > current {
        UpdateStatus::Available {
            version: release.tag_name,
            url: release.html_url,
        }
    } else {
        UpdateStatus::UpToDate
    }
}

pub(crate) fn update_message(status: &UpdateStatus, palette: Palette) -> StatusMessage {
    let (text, color, url) = match status {
        UpdateStatus::Checking => (
            "Checking for updates\u{2026}".to_string(),
            palette.comment,
            None,
        ),
        UpdateStatus::UpToDate => ("You're up to date".to_string(), palette.comment, None),
        UpdateStatus::Available { version, url } => (
            format!("Version {version} is available"),
            palette.result,
            Some(url.clone()),
        ),
        UpdateStatus::Failed => (
            "Couldn't check for updates".to_string(),
            palette.error,
            None,
        ),
    };
    StatusMessage { text, color, url }
}

/// Only `https://github.com/...` is opened. The URL comes from a network
/// response, and the trailing `/` also rejects look-alikes such as
/// `https://github.com@evil.example` and `https://github.com.evil.example`.
pub(crate) fn is_release_url(url: &str) -> bool {
    url.starts_with("https://github.com/")
}

/// Opens `url` with the OS's own launcher, passing it as one argument and
/// never through a shell (`cmd /C start` would re-parse it).
pub(crate) fn open_url(url: &str) {
    if !is_release_url(url) {
        return;
    }
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

pub(crate) enum UpdateStatus {
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed,
}

impl SoosApp {
    /// Check for a newer release on a background thread, unless a check is
    /// already running.
    pub(crate) fn check_for_update(&mut self, ctx: &egui::Context) {
        if matches!(self.update_status, Some(UpdateStatus::Checking)) {
            return;
        }
        self.update_status = Some(UpdateStatus::Checking);
        let tx = self.tx.clone();
        let repaint_ctx = ctx.clone();
        std::thread::spawn(move || {
            let status = check_latest_release();
            let _ = tx.send(AppEvent::UpdateChecked(status));
            repaint_ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_compares_numerically_not_lexically() {
        assert!(parse_version("0.9.0") < parse_version("0.10.0"));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("not-a-version"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn latest_release_url_derives_from_cargo_repository() {
        let url = latest_release_url().expect("repository field is a github.com URL");
        assert!(url.starts_with("https://api.github.com/repos/"));
        assert!(url.ends_with("/releases/latest"));
    }

    #[test]
    fn is_release_url_pins_scheme_and_host() {
        assert!(is_release_url(
            "https://github.com/marelevn/soos/releases/tag/v1.0"
        ));

        assert!(!is_release_url("http://github.com/owner/repo"));
        assert!(!is_release_url("file:///etc/passwd"));
        assert!(!is_release_url("javascript:alert(1)"));
        assert!(!is_release_url("smb://x/y"));
        assert!(!is_release_url("https://"));
        assert!(!is_release_url("https://github.com@evil.example/x"));
        assert!(!is_release_url("https://github.com:443@evil.example/x"));
        assert!(!is_release_url("https://github.com.evil.example/x"));
    }
}
