//! Running `adb` against one device.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use crate::error::{Error, Result};

const ENABLED_SERVICES: &str = "enabled_accessibility_services";

/// `adb` bound to one device's serial number.
#[derive(Debug, Clone)]
pub struct Adb {
    pub bin: PathBuf,
    pub serial: String,
}

impl Adb {
    pub fn new(bin: PathBuf, serial: impl Into<String>) -> Self {
        Adb {
            bin,
            serial: serial.into(),
        }
    }

    /// Runs adb with these arguments for this device and returns what it printed.
    pub async fn raw(&self, args: &[&str]) -> Result<String> {
        let out = Command::new(&self.bin)
            .arg("-s")
            .arg(&self.serial)
            .args(args)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| Error::Adb(format!("adb could not run: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let detail = if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            };
            return Err(Error::Adb(format!("{} ({})", detail, args.join(" "))));
        }
        Ok(stdout)
    }

    /// Runs a shell command on the device and returns what it printed.
    pub async fn shell(&self, command: &str) -> Result<String> {
        Ok(self.raw(&["shell", command]).await?.trim_end().to_string())
    }

    /// True once adb can reach the device and Android reports it has finished booting.
    pub async fn boot_completed(&self) -> bool {
        matches!(self.shell("getprop sys.boot_completed").await, Ok(v) if v.trim() == "1")
    }

    /// Waits until Android has finished booting, up to `timeout`.
    pub async fn wait_for_boot(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if self.boot_completed().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        false
    }

    /// Installs or updates an app, granting the runtime permissions it asks for.
    pub async fn install(&self, apk: &Path) -> Result<()> {
        let apk = apk.to_string_lossy();
        let out = self.raw(&["install", "-r", "-g", "-t", &apk]).await?;
        if out.contains("Success") {
            Ok(())
        } else {
            Err(Error::Adb(out.trim().to_string()))
        }
    }

    pub async fn uninstall(&self, package: &str) -> Result<()> {
        self.raw(&["uninstall", package]).await.map(|_| ())
    }

    pub async fn is_installed(&self, package: &str) -> Result<bool> {
        let out = self.shell(&format!("pm list packages {package}")).await?;
        Ok(out
            .lines()
            .any(|line| line.trim() == format!("package:{package}")))
    }

    /// The installed version code of a package, if it is installed.
    pub async fn version_code(&self, package: &str) -> Result<Option<u64>> {
        let dump = self.shell(&format!("dumpsys package {package}")).await?;
        Ok(dump
            .lines()
            .filter_map(|line| line.trim().strip_prefix("versionCode="))
            .filter_map(|rest| rest.split_whitespace().next()?.parse().ok())
            .max())
    }

    pub async fn setting(&self, namespace: &str, key: &str) -> Result<Option<String>> {
        let value = self
            .shell(&format!("settings get {namespace} {key}"))
            .await?;
        let value = value.trim();
        Ok((value != "null" && !value.is_empty()).then(|| value.to_string()))
    }

    pub async fn put_setting(&self, namespace: &str, key: &str, value: &str) -> Result<()> {
        self.shell(&format!(
            "settings put {namespace} {key} {}",
            shell_quote(value)
        ))
        .await
        .map(|_| ())
    }

    /// The accessibility services Android currently has turned on.
    pub async fn enabled_services(&self) -> Result<Vec<String>> {
        Ok(self
            .setting("secure", ENABLED_SERVICES)
            .await?
            .map(|v| split_components(&v))
            .unwrap_or_default())
    }

    /// Turns on accessibility services, keeping the ones already on. Returns the
    /// services that were off and are now on.
    pub async fn enable_services(&self, components: &[String]) -> Result<Vec<String>> {
        let mut enabled = self.enabled_services().await?;
        let mut turned_on = Vec::new();
        for component in components {
            if !enabled.iter().any(|c| same_component(c, component)) {
                enabled.push(component.clone());
                turned_on.push(component.clone());
            }
        }
        if !turned_on.is_empty() {
            self.put_setting("secure", ENABLED_SERVICES, &enabled.join(":"))
                .await?;
        }
        self.put_setting("secure", "accessibility_enabled", "1")
            .await?;
        Ok(turned_on)
    }

    /// Turns on accessibility services and waits until Android is running them.
    ///
    /// Right after an app is installed or updated, Android may still be
    /// rescanning it, and removes services it doesn't know yet from the
    /// enabled list. So this checks what Android is actually running and puts
    /// back any service that was dropped, until all are running or `timeout`
    /// passes. Returns the services that had been off.
    pub async fn ensure_services(
        &self,
        components: &[String],
        timeout: Duration,
    ) -> Result<Vec<String>> {
        let turned_on = self.enable_services(components).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let running = self.running_services().await?;
            let missing: Vec<String> = components
                .iter()
                .filter(|c| !running.iter().any(|r| same_component(r, c)))
                .cloned()
                .collect();
            if missing.is_empty() {
                return Ok(turned_on);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Adb(format!(
                    "Android did not turn on {}. It may have stopped working; check the device's log.",
                    missing.join(", ")
                )));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            self.enable_services(&missing).await?;
        }
    }

    /// The accessibility services Android is running, from `dumpsys accessibility`.
    pub async fn running_services(&self) -> Result<Vec<String>> {
        let dump = self.shell("dumpsys accessibility").await?;
        Ok(parse_running_services(&dump))
    }

    pub async fn disable_service(&self, component: &str) -> Result<()> {
        let enabled: Vec<String> = self
            .enabled_services()
            .await?
            .into_iter()
            .filter(|c| !same_component(c, component))
            .collect();
        self.put_setting("secure", ENABLED_SERVICES, &enabled.join(":"))
            .await
    }
}

/// Reads the `Enabled services:{{a/b}, {c/d}}` line of `dumpsys accessibility`.
fn parse_running_services(dump: &str) -> Vec<String> {
    let Some(line) = dump
        .lines()
        .find_map(|l| l.trim().strip_prefix("Enabled services:"))
    else {
        return Vec::new();
    };
    line.split(['{', '}', ','])
        .map(str::trim)
        .filter(|part| part.contains('/'))
        .map(String::from)
        .collect()
}

fn split_components(value: &str) -> Vec<String> {
    value
        .split(':')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from)
        .collect()
}

/// True if two `package/class` components name the same service. Android may
/// store `pkg/.Short` or `pkg/pkg.Full` for the same one.
pub fn same_component(a: &str, b: &str) -> bool {
    expand_component(a) == expand_component(b)
}

fn expand_component(component: &str) -> String {
    match component.split_once('/') {
        Some((pkg, cls)) if cls.starts_with('.') => format!("{pkg}/{pkg}{cls}"),
        _ => component.to_string(),
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_short_and_long_components() {
        assert!(same_component("com.a/.Svc", "com.a/com.a.Svc"));
        assert!(!same_component("com.a/.Svc", "com.b/com.a.Svc"));
    }

    #[test]
    fn reads_running_services() {
        let dump = "User state[\n     Bound services:{Service[label=X]}\n     Enabled services:{{a.b/a.b.S}, {c/.D}}\n";
        assert_eq!(parse_running_services(dump), vec!["a.b/a.b.S", "c/.D"]);
        assert!(parse_running_services("Enabled services:{}").is_empty());
    }

    #[test]
    fn splits_service_list() {
        assert_eq!(split_components("a/b: c/d :"), vec!["a/b", "c/d"]);
    }

    #[test]
    fn quotes_for_the_shell() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
