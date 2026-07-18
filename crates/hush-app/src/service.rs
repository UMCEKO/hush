//! systemd --user unit management for the daemon: install, autostart, restart.

use std::path::PathBuf;

use crate::ipc::send_shutdown;

pub(crate) fn service_enabled() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "hush.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn unit_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/systemd/user/hush.service")
}

/// `LD_LIBRARY_PATH` for the daemon unit — the resolved SDK's lib dirs + host
/// driver. Empty if no SDK is installed yet (the unit still installs; the daemon
/// stays down until setup provisions the SDK, then the unit is regenerated).
fn sdk_lib_path() -> String {
    hush_core::sdk::ld_library_path().unwrap_or_default()
}

/// Write the systemd *user* unit (no root needed — it lives under `~/.config`),
/// pointing at the sibling `hushd` binary with the SDK on its library path.
fn install_unit() -> std::io::Result<()> {
    let hushd = std::env::current_exe()?.with_file_name("hushd");
    let unit = format!(
        "[Unit]\n\
         Description=HUSH denoiser daemon\n\
         After=pipewire.service wireplumber.service\n\
         Wants=pipewire.service\n\n\
         [Service]\n\
         Type=simple\n\
         ExecStart={hushd}\n\
         Environment=LD_LIBRARY_PATH={libs}\n\
         Restart=on-failure\n\
         RestartSec=2\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        hushd = hushd.display(),
        libs = sdk_lib_path(),
    );
    let path = unit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, unit)?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

/// Toggle launch-at-login. All `--user` scope — never needs sudo. Self-installs
/// the unit on first enable so the toggle works without a separate install step.
pub(crate) fn set_autostart(on: bool) {
    if on {
        let _ = install_unit();
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "enable", "hush.service"])
            .status();
    } else {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "hush.service"])
            .status();
    }
}

/// Restart the denoiser so it re-resolves the model / GPU. Prefers the systemd
/// user unit; if that isn't active (sibling-spawned daemon), asks it to exit so
/// `spawn_ipc_sync`'s reconnect loop respawns a fresh one.
pub(crate) fn restart_engine() {
    // Regenerate the unit so a freshly-provisioned/relocated SDK lands in the
    // service's LD_LIBRARY_PATH before it starts.
    if unit_path().exists() {
        let _ = install_unit();
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "restart", "hush.service"])
        .status();
    let active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "hush.service"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !active {
        send_shutdown();
    }
}
