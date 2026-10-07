//! Reading app packages (APKs): the package name, and the parts that need the
//! user's permission to run, such as accessibility services.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::platform::NoConsole;

const ANDROID_NS: &str = "http://schemas.android.com/apk/res/android:";

/// What kind of special service an app contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Accessibility,
    InputMethod,
    NotificationListener,
    /// A device administrator, which can lock the device or wipe it.
    DeviceAdmin,
}

impl ServiceKind {
    fn from_permission(permission: &str) -> Option<Self> {
        match permission {
            "android.permission.BIND_ACCESSIBILITY_SERVICE" => Some(Self::Accessibility),
            "android.permission.BIND_INPUT_METHOD" => Some(Self::InputMethod),
            "android.permission.BIND_NOTIFICATION_LISTENER_SERVICE" => {
                Some(Self::NotificationListener)
            }
            "android.permission.BIND_DEVICE_ADMIN" => Some(Self::DeviceAdmin),
            _ => None,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Accessibility => "an accessibility service",
            Self::InputMethod => "a keyboard",
            Self::NotificationListener => "a notification listener",
            Self::DeviceAdmin => "a device administrator",
        }
    }
}

/// A service (or, for a device administrator, a receiver) declared in an
/// app's manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub kind: ServiceKind,
    /// The full class name.
    pub class: String,
}

impl Service {
    /// As Android names it in settings, `package/class`.
    pub fn component(&self, package: &str) -> String {
        format!("{package}/{}", self.class)
    }

    /// The class name without its package, such as "TalkBackService".
    pub fn short_name(&self) -> &str {
        self.class.rsplit('.').next().unwrap_or(&self.class)
    }
}

/// What AAE needs to know about an app package.
#[derive(Debug, Clone)]
pub struct ApkInfo {
    pub path: PathBuf,
    pub package: String,
    pub version_code: Option<u64>,
    pub services: Vec<Service>,
}

impl ApkInfo {
    /// Reads an APK with `aapt2` from the SDK's build tools.
    pub fn read(aapt2: &Path, apk: &Path) -> Result<Self> {
        let fail = |reason: String| Error::Apk {
            path: apk.to_path_buf(),
            reason,
        };
        let run = |args: &[&str]| -> Result<String> {
            let out = Command::new(aapt2)
                .no_console()
                .args(args)
                .arg(apk)
                .output()
                .map_err(|e| fail(format!("aapt2 could not run: {e}")))?;
            if !out.status.success() {
                return Err(fail(
                    String::from_utf8_lossy(&out.stderr).trim().to_string(),
                ));
            }
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let package = run(&["dump", "packagename"])?.trim().to_string();
        let manifest = run(&["dump", "xmltree", "--file", "AndroidManifest.xml"])?;
        let services = parse_services(&manifest, &package);
        let version_code = parse_plain_version(&manifest);
        Ok(ApkInfo {
            path: apk.to_path_buf(),
            package,
            version_code,
            services,
        })
    }

    /// Accessibility services, as `package/class` components ready for Android's settings.
    pub fn accessibility_components(&self) -> Vec<String> {
        self.services
            .iter()
            .filter(|s| s.kind == ServiceKind::Accessibility)
            .map(|s| format!("{}/{}", self.package, s.class))
            .collect()
    }
}

/// Finds `<service>` elements guarded by a binding permission in `aapt2 dump
/// xmltree` output. Elements nest by indentation; attributes are `A:` lines.
fn parse_services(tree: &str, package: &str) -> Vec<Service> {
    let mut services = Vec::new();
    let mut current: Option<(usize, Option<String>, Option<String>)> = None;

    let mut finish = |entry: Option<(usize, Option<String>, Option<String>)>| {
        if let Some((_, Some(name), Some(permission))) = entry {
            if let Some(kind) = ServiceKind::from_permission(&permission) {
                let class = if name.starts_with('.') {
                    format!("{package}{name}")
                } else {
                    name
                };
                services.push(Service { kind, class });
            }
        }
    };

    for line in tree.lines() {
        let indent = line.len() - line.trim_start().len();
        let text = line.trim_start();
        if let Some(element) = text.strip_prefix("E: ") {
            if current.as_ref().is_some_and(|(depth, ..)| indent <= *depth) {
                finish(current.take());
            }
            let bound = ["service", "receiver"];
            if bound
                .iter()
                .any(|e| element == *e || element.starts_with(&format!("{e} ")))
            {
                current = Some((indent, None, None));
            }
        } else if let Some(attr) = text.strip_prefix("A: ") {
            let Some((depth, name, permission)) = current.as_mut() else {
                continue;
            };
            // Only the service's own attributes, not those of its children.
            if indent > *depth + 2 {
                continue;
            }
            if let Some(value) = attribute(attr, "name") {
                *name = Some(value);
            } else if let Some(value) = attribute(attr, "permission") {
                *permission = Some(value);
            }
        }
    }
    finish(current.take());
    services
}

/// aapt2 prints integer attributes without quotes, such as
/// `android:versionCode(0x0101021b)=2`.
fn parse_plain_version(tree: &str) -> Option<u64> {
    let start = tree.find(&format!("{ANDROID_NS}versionCode("))?;
    let rest = &tree[start..];
    let value = rest.split_once(")=")?.1;
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Reads `ns:name(0x...)="value" (Raw: ...)` when the attribute has the given name.
fn attribute(attr: &str, name: &str) -> Option<String> {
    let rest = attr.strip_prefix(ANDROID_NS)?.strip_prefix(name)?;
    let rest = rest.strip_prefix('(')?;
    let (_, rest) = rest.split_once(")=\"")?;
    let (value, _) = rest.split_once('"')?;
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE: &str = r#"N: android=http://schemas.android.com/apk/res/android (line=2)
  E: manifest (line=2)
    E: application (line=10)
      E: service (line=160)
        A: http://schemas.android.com/apk/res/android:label(0x01010001)=@0x7f120f05
        A: http://schemas.android.com/apk/res/android:name(0x01010003)="com.google.android.marvin.talkback.TalkBackService" (Raw: "com.google.android.marvin.talkback.TalkBackService")
        A: http://schemas.android.com/apk/res/android:permission(0x01010006)="android.permission.BIND_ACCESSIBILITY_SERVICE" (Raw: "android.permission.BIND_ACCESSIBILITY_SERVICE")
          E: intent-filter (line=166)
            E: action (line=167)
              A: http://schemas.android.com/apk/res/android:name(0x01010003)="android.accessibilityservice.AccessibilityService" (Raw: "android.accessibilityservice.AccessibilityService")
      E: service (line=180)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)=".Plain" (Raw: ".Plain")
      E: service (line=190)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)=".ime.Keys" (Raw: ".ime.Keys")
        A: http://schemas.android.com/apk/res/android:permission(0x01010006)="android.permission.BIND_INPUT_METHOD" (Raw: "android.permission.BIND_INPUT_METHOD")
"#;

    #[test]
    fn finds_device_administrators() {
        let tree = r#"  E: manifest (line=2)
    E: application (line=10)
      E: receiver (line=20)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)=".Admin" (Raw: ".Admin")
        A: http://schemas.android.com/apk/res/android:permission(0x01010006)="android.permission.BIND_DEVICE_ADMIN" (Raw: "android.permission.BIND_DEVICE_ADMIN")
      E: receiver (line=30)
        A: http://schemas.android.com/apk/res/android:name(0x01010003)=".Boot" (Raw: ".Boot")
"#;
        let services = parse_services(tree, "com.example");
        assert_eq!(
            services,
            vec![Service {
                kind: ServiceKind::DeviceAdmin,
                class: "com.example.Admin".into()
            }]
        );
        assert_eq!(
            services[0].component("com.example"),
            "com.example/com.example.Admin"
        );
        assert_eq!(services[0].short_name(), "Admin");
    }

    #[test]
    fn reads_version_code() {
        let tree = "E: manifest\n  A: http://schemas.android.com/apk/res/android:versionCode(0x0101021b)=2\n";
        assert_eq!(parse_plain_version(tree), Some(2));
    }

    #[test]
    fn finds_bound_services() {
        let services = parse_services(TREE, "com.example");
        assert_eq!(
            services,
            vec![
                Service {
                    kind: ServiceKind::Accessibility,
                    class: "com.google.android.marvin.talkback.TalkBackService".into()
                },
                Service {
                    kind: ServiceKind::InputMethod,
                    class: "com.example.ime.Keys".into()
                },
            ]
        );
    }
}
