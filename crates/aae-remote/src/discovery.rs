//! Announces the server on the local network, so the phone finds it without
//! anyone typing an address: as `_aae._tcp`, with the certificate's
//! fingerprint, which the phone checks against the one it paired with.

use mdns_sd::{ServiceDaemon, ServiceInfo};

pub const SERVICE: &str = "_aae._tcp.local.";

/// Keeps the announcement up while it's alive.
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}

pub fn announce(name: &str, port: u16, fingerprint: &str) -> Result<Announcement, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    // Host names can't have spaces or apostrophes.
    let host: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let properties = [
        ("fingerprint", fingerprint),
        ("version", env!("CARGO_PKG_VERSION")),
    ];
    let info = ServiceInfo::new(
        SERVICE,
        name,
        &format!("{host}.local."),
        "",
        port,
        &properties[..],
    )
    .map_err(|e| e.to_string())?
    .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info).map_err(|e| e.to_string())?;
    Ok(Announcement { daemon, fullname })
}

impl Drop for Announcement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// This computer's name, as people know it, for phones to list.
pub fn computer_name() -> String {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    };
    let name = if cfg!(target_os = "macos") {
        run("scutil", &["--get", "ComputerName"])
    } else if cfg!(windows) {
        std::env::var("COMPUTERNAME").ok()
    } else {
        run("hostname", &[])
    };
    name.unwrap_or_else(|| "AAE".into())
}
