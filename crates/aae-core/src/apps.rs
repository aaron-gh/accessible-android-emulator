//! Managing a device's installed apps: listing them by the names people see,
//! opening, stopping, clearing and removing them, and their permissions and
//! special access. Names and permissions come from AAE's helper, which can
//! read what adb's listings leave out.

use serde::Deserialize;

use crate::adb::{Adb, shell_quote};
use crate::error::{Error, Result};

const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const LIST_APPS: &str = "io.github.aaron_gh.aae.helper.LIST_APPS";
const APP_PERMISSIONS: &str = "io.github.aaron_gh.aae.helper.APP_PERMISSIONS";

/// An installed app.
#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub package: String,
    /// The name people see, such as "Gmail".
    pub label: String,
    #[serde(default)]
    pub version: String,
    /// Part of Android or the device, rather than installed by someone.
    #[serde(default)]
    pub system: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// It has an icon to open it with.
    #[serde(default)]
    pub launchable: bool,
}

fn yes() -> bool {
    true
}

/// A permission an app asks the user for.
#[derive(Debug, Clone, Deserialize)]
pub struct Permission {
    /// Such as `android.permission.CAMERA`.
    pub name: String,
    /// The name people see, such as "take pictures and record video".
    pub label: String,
    pub granted: bool,
}

/// Access an app is given in Android's special settings, not as a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Unrestricted battery use: not held back by battery optimisation.
    Battery,
    /// Display over other apps.
    Overlay,
    /// Usage access: seeing which apps are used.
    Usage,
    /// Modify system settings.
    WriteSettings,
}

impl Access {
    pub const ALL: [Access; 4] = [
        Access::Battery,
        Access::Overlay,
        Access::Usage,
        Access::WriteSettings,
    ];

    pub fn describe(self) -> &'static str {
        match self {
            Access::Battery => "Unrestricted battery use",
            Access::Overlay => "Display over other apps",
            Access::Usage => "Usage access",
            Access::WriteSettings => "Modify system settings",
        }
    }

    /// The app operation behind it, for those that are one.
    fn app_op(self) -> Option<&'static str> {
        match self {
            Access::Battery => None,
            Access::Overlay => Some("SYSTEM_ALERT_WINDOW"),
            Access::Usage => Some("GET_USAGE_STATS"),
            Access::WriteSettings => Some("WRITE_SETTINGS"),
        }
    }
}

/// Asks AAE's helper, and returns the JSON it answers with.
async fn ask_helper(adb: &Adb, action: &str, extras: &str) -> Result<String> {
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {action}{extras}"
        ))
        .await?;
    if !out.contains("result=1") {
        return Err(Error::Adb(
            "AAE's helper didn't answer. Starting the device again updates it.".into(),
        ));
    }
    out.split_once("data=\"")
        .and_then(|(_, rest)| rest.rsplit_once('"'))
        .map(|(json, _)| json.to_string())
        .ok_or_else(|| Error::Adb("AAE's helper sent no answer.".into()))
}

/// The installed apps, by name. With `system`, Android's own apps too.
pub async fn list(adb: &Adb, system: bool) -> Result<Vec<App>> {
    let json = ask_helper(adb, LIST_APPS, "").await?;
    let mut apps: Vec<App> = serde_json::from_str(&json)
        .map_err(|e| Error::Adb(format!("AAE's helper sent a list AAE can't read: {e}")))?;
    apps.retain(|a| system || !a.system);
    apps.sort_by_key(|a| a.label.to_lowercase());
    Ok(apps)
}

/// The permissions an app asks the user for, and whether each is granted.
pub async fn permissions(adb: &Adb, package: &str) -> Result<Vec<Permission>> {
    let json = ask_helper(adb, APP_PERMISSIONS, &format!(" --es package {package}")).await?;
    serde_json::from_str(&json)
        .map_err(|e| Error::Adb(format!("AAE's helper sent permissions AAE can't read: {e}")))
}

/// Grants or revokes one permission.
pub async fn set_permission(adb: &Adb, package: &str, permission: &str, grant: bool) -> Result<()> {
    let verb = if grant { "grant" } else { "revoke" };
    let out = adb
        .shell(&format!("pm {verb} {package} {permission}"))
        .await?;
    if out.trim().is_empty() {
        Ok(())
    } else {
        Err(Error::Adb(out.trim().to_string()))
    }
}

/// Grants every permission the app asks for. Returns how many were granted.
pub async fn grant_all(adb: &Adb, package: &str) -> Result<usize> {
    let mut granted = 0;
    for permission in permissions(adb, package).await? {
        if !permission.granted {
            set_permission(adb, package, &permission.name, true).await?;
            granted += 1;
        }
    }
    Ok(granted)
}

/// Whether the app has a kind of special access.
pub async fn has_access(adb: &Adb, package: &str, access: Access) -> Result<bool> {
    match access.app_op() {
        None => {
            let list = adb.shell("cmd deviceidle whitelist").await?;
            Ok(list.lines().any(|l| l.split(',').nth(1) == Some(package)))
        }
        Some(op) => {
            let out = adb.shell(&format!("appops get {package} {op}")).await?;
            Ok(out.contains(": allow"))
        }
    }
}

/// Gives or takes away a kind of special access.
pub async fn set_access(adb: &Adb, package: &str, access: Access, on: bool) -> Result<()> {
    match access.app_op() {
        None => {
            let sign = if on { '+' } else { '-' };
            adb.shell(&format!("cmd deviceidle whitelist {sign}{package}"))
                .await?;
        }
        Some(op) => {
            let mode = if on { "allow" } else { "default" };
            adb.shell(&format!("appops set {package} {op} {mode}"))
                .await?;
        }
    }
    Ok(())
}

/// Opens an app as its icon would.
pub async fn open(adb: &Adb, package: &str) -> Result<()> {
    let found = adb
        .shell(&format!(
            "cmd package resolve-activity --brief -a android.intent.action.MAIN \
             -c android.intent.category.LAUNCHER {package}"
        ))
        .await
        .unwrap_or_default();
    match found
        .lines()
        .last()
        .map(str::trim)
        .filter(|l| l.contains('/'))
    {
        Some(component) => open_activity(adb, component).await,
        None => Err(Error::Adb(format!(
            "{package} has no screen to open from an icon."
        ))),
    }
}

/// Opens one of an app's screens by name, such as `com.example/.Settings`.
pub async fn open_activity(adb: &Adb, component: &str) -> Result<()> {
    let out = adb.shell(&format!("am start -n {component}")).await?;
    if out.contains("Error") {
        Err(Error::Adb(out.trim().to_string()))
    } else {
        Ok(())
    }
}

/// Stops an app, as Force Stop in its settings does.
pub async fn force_stop(adb: &Adb, package: &str) -> Result<()> {
    adb.shell(&format!("am force-stop {package}")).await?;
    Ok(())
}

/// Deletes an app's data, as if it had just been installed.
pub async fn clear_data(adb: &Adb, package: &str) -> Result<()> {
    let out = adb.shell(&format!("pm clear {package}")).await?;
    if out.contains("Success") {
        Ok(())
    } else {
        Err(Error::Adb(out.trim().to_string()))
    }
}

/// An intent to send: what to do, on what, by whom.
#[derive(Debug, Clone, Default)]
pub struct Intent {
    /// Such as `android.intent.action.VIEW`. None sends none.
    pub action: Option<String>,
    /// A link or other address, such as `https://example.com` or `myapp://home`.
    pub data: Option<String>,
    /// An app's package, or one of its screens or receivers as
    /// `package/class`. None lets Android choose.
    pub target: Option<String>,
    /// Text extras, as (key, value).
    pub extras: Vec<(String, String)>,
    /// Send as a broadcast, not to open a screen.
    pub broadcast: bool,
}

/// Opens a link, in the given app or whichever Android chooses.
pub async fn open_link(adb: &Adb, link: &str, app: Option<&str>) -> Result<String> {
    send(
        adb,
        &Intent {
            action: Some("android.intent.action.VIEW".into()),
            data: Some(link.to_string()),
            target: app.map(String::from),
            ..Default::default()
        },
    )
    .await
}

/// Sends an intent, and returns what Android said.
pub async fn send(adb: &Adb, intent: &Intent) -> Result<String> {
    let mut command = String::from(if intent.broadcast {
        "am broadcast"
    } else {
        "am start"
    });
    if let Some(action) = &intent.action {
        command.push_str(&format!(" -a {}", shell_quote(action)));
    }
    if let Some(data) = &intent.data {
        command.push_str(&format!(" -d {}", shell_quote(data)));
    }
    for (key, value) in &intent.extras {
        command.push_str(&format!(
            " --es {} {}",
            shell_quote(key),
            shell_quote(value)
        ));
    }
    match intent.target.as_deref() {
        Some(component) if component.contains('/') => {
            command.push_str(&format!(" -n {}", shell_quote(component)))
        }
        Some(package) => command.push_str(&format!(" -p {}", shell_quote(package))),
        None => {}
    }
    let out = adb.shell(&command).await?;
    // am says so when nothing could handle it.
    if out.contains("unable to resolve") {
        return Err(Error::Adb(
            "Nothing on the device can handle that. Check the address, or install the app it's for."
                .into(),
        ));
    }
    if out.contains("Error") {
        return Err(Error::Adb(out.trim().to_string()));
    }
    Ok(out.trim().to_string())
}
