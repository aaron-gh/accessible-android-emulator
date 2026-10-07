//! Setting devices up and putting them back: starting and stopping,
//! creating and copying, installing apps and choosing their parts,
//! permissions, accessibility services, and snapshots for repeatable
//! starting points. Tools that can't be undone are in their own group,
//! offered only with --allow-destructive.

use std::sync::{Arc, Mutex};

use aae_ffi::{AppChoice, AppPartKind, DeviceProfile, ProgressListener};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData, schemars, tool, tool_router};
use serde::{Deserialize, Serialize};

use crate::{AaeServer, DeviceParam, json, respond, text};

/// Collects the progress a long operation reports, to say what happened.
#[derive(Default)]
struct Steps(Mutex<Vec<String>>);

impl ProgressListener for Steps {
    fn progress(&self, message: String) {
        self.0.lock().unwrap().push(message);
    }
}

impl Steps {
    fn said(&self) -> String {
        self.0.lock().unwrap().join(" ")
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CreateParam {
    /// The new device's name.
    pub name: String,
    /// An installed Android version, as list_android_versions describes it,
    /// or its API level, such as 37 or 36.1, or a preview's name, such as
    /// 37.2-beta3. Downloading a version needs its licence accepted by the
    /// user, so it's done in AAE's app, not here.
    pub version: String,
    /// "small-phone", "phone" (the default) or "tablet".
    pub size: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct VersionsParam {
    /// Also list the versions Google offers to download, not just those
    /// installed.
    pub all: Option<bool>,
    /// With `all`, include previews of upcoming Android releases too.
    pub previews: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NameParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The new name.
    pub new_name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct InstallParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The APK's path on this computer.
    pub path: String,
    /// Which of the app's special parts to turn on, when it has any not
    /// chosen before on this device: "accessibility" (the default, as AAE's
    /// app suggests) turns on accessibility services; "all" also keyboards,
    /// notification listeners and device administrators; "none" leaves
    /// them all off. Earlier choices on this device are kept.
    pub parts: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ListAppsParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// Include Android's own apps, not just the ones installed.
    pub system: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct AppParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The app, by the name people see or its package.
    pub app: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct PermissionParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The app, by the name people see or its package.
    pub app: String,
    /// The permission, such as android.permission.CAMERA, or "all" to grant
    /// every permission the app asks for.
    pub permission: String,
    pub granted: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ServiceParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The service, by its name, such as TalkBack, or its package or component.
    pub service: String,
    /// On or off. Turning a screen reader on makes it the device's screen
    /// reader, turning off the one it had.
    pub on: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SnapshotParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// The snapshot's name.
    pub name: String,
    /// Notes to keep with a new snapshot, such as what it's for.
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ShellParam {
    /// The device's name. Leave it out when only one device is running.
    pub device: Option<String>,
    /// A command for the device's shell, run as the shell user. It's
    /// stopped after two minutes.
    pub command: String,
}

#[derive(Serialize)]
struct Version {
    description: String,
    api: u32,
    installed: bool,
    download_size: String,
}

#[derive(Serialize)]
struct App {
    name: String,
    package: String,
    version: String,
    system: bool,
    enabled: bool,
}

#[derive(Serialize)]
struct Permission {
    name: String,
    description: String,
    granted: bool,
}

#[derive(Serialize)]
struct Permissions {
    permissions: Vec<Permission>,
    special_access: Vec<Permission>,
}

#[derive(Serialize)]
struct Service {
    name: String,
    component: String,
    description: String,
    screen_reader: bool,
    on: bool,
}

#[derive(Serialize)]
struct Snapshot {
    name: String,
    notes: String,
    taken: Option<String>,
    size: String,
    loaded: bool,
    usable: bool,
}

#[tool_router(router = manage_tools, vis = "pub(crate)")]
impl AaeServer {
    /// Starts a device and waits until it's ready, with its screen reader
    /// on. The first start sets it up, which takes a few minutes.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn start_device(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let device = self.device(p.device.as_deref())?;
                if device.running {
                    return text(format!("{} is already running.", device.name));
                }
                let steps = Arc::new(Steps::default());
                self.engine
                    .start_device(
                        device.id.clone(),
                        self.engine.default_screen_reader(),
                        true,
                        steps.clone(),
                    )
                    .await?;
                self.forget(&device.id).await;
                text(format!("{} is ready. {}", device.name, steps.said()))
            }
            .await,
        )
    }

    /// Stops a device, saving its state so it starts quickly next time.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn stop_device(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let device = self.device(p.device.as_deref())?;
                self.forget(&device.id).await;
                if !device.running {
                    return text(format!("{} isn't running.", device.name));
                }
                self.engine.stop_device(device.id.clone()).await?;
                text(format!("{} is stopped.", device.name))
            }
            .await,
        )
    }

    /// Restarts Android on a running device, keeping everything on it.
    #[tool(annotations(destructive_hint = false))]
    async fn restart_device(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let device = self.device(p.device.as_deref())?;
                self.forget(&device.id).await;
                let steps = Arc::new(Steps::default());
                self.engine
                    .restart_device(device.id.clone(), steps.clone())
                    .await?;
                text(format!("{} has restarted. {}", device.name, steps.said()))
            }
            .await,
        )
    }

    /// Lists the Android versions installed, which devices can be created
    /// from, and with `all`, those Google offers to download too.
    #[tool(annotations(read_only_hint = true))]
    async fn list_android_versions(
        &self,
        Parameters(p): Parameters<VersionsParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let versions = self
                    .engine
                    .versions(false, p.previews.unwrap_or(false))
                    .await?;
                let all = p.all.unwrap_or(false);
                json(
                    &versions
                        .into_iter()
                        .filter(|v| all || v.installed)
                        .map(|v| Version {
                            description: v.description,
                            api: v.api,
                            installed: v.installed,
                            download_size: v.size,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            .await,
        )
    }

    /// Creates a device from an installed Android version. start_device
    /// then starts it and sets it up.
    #[tool(annotations(destructive_hint = false))]
    async fn create_device(
        &self,
        Parameters(p): Parameters<CreateParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let images = self.engine.images();
                let wanted = p.version.trim().to_lowercase();
                let image = images
                    .iter()
                    .find(|i| i.description.to_lowercase() == wanted)
                    .or_else(|| {
                        images.iter().find(|i| {
                            i.release.eq_ignore_ascii_case(&wanted)
                                || aae_core::sdk::parse_api_level(&wanted)
                                    .zip(aae_core::sdk::parse_api_level(&i.release))
                                    .is_some_and(|(a, b)| a == b)
                        })
                    })
                    .or_else(|| images.iter().find(|i| i.description.to_lowercase().contains(&wanted)))
                    .ok_or_else(|| {
                        let installed: Vec<&str> = images.iter().map(|i| i.description.as_str()).collect();
                        anyhow::anyhow!(
                            "No installed Android version matches \"{}\". Installed: {}. Others can be downloaded in AAE's app, which asks the user to accept Google's licence.",
                            p.version,
                            if installed.is_empty() { "none".into() } else { installed.join("; ") }
                        )
                    })?;
                let profile = match p.size.as_deref().map(|s| s.to_ascii_lowercase().replace([' ', '_'], "-")) {
                    Some(ref s) if s == "small-phone" => DeviceProfile::SmallPhone,
                    Some(ref s) if s == "tablet" => DeviceProfile::Tablet,
                    None => DeviceProfile::Phone,
                    Some(ref s) if s == "phone" => DeviceProfile::Phone,
                    Some(other) => anyhow::bail!("\"{other}\" isn't a size. Use small-phone, phone or tablet."),
                };
                let device = self.engine.create_device(p.name.clone(), image.sysdir.clone(), profile)?;
                text(format!(
                    "Created {}, {}. Start it with start_device.",
                    device.name, device.android
                ))
            }
            .await,
        )
    }

    /// Copies a stopped device, with its apps and data, under a new name.
    #[tool(annotations(destructive_hint = false))]
    async fn copy_device(
        &self,
        Parameters(p): Parameters<NameParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let device = self.device(p.device.as_deref())?;
                let copy = self.engine.clone_device(device.id.clone(), p.new_name)?;
                text(format!("Created {}, a copy of {}.", copy.name, device.name))
            }
            .await,
        )
    }

    /// Installs an app from an APK on this computer, keeping its data if
    /// it's already installed, and turns on its accessibility services and
    /// other special parts as asked.
    #[tool(annotations(destructive_hint = false))]
    async fn install_app(
        &self,
        Parameters(p): Parameters<InstallParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let policy = p
                    .parts
                    .as_deref()
                    .unwrap_or("accessibility")
                    .to_ascii_lowercase();
                if !matches!(policy.as_str(), "accessibility" | "all" | "none") {
                    anyhow::bail!(
                        "\"{policy}\" isn't a choice for parts. Use accessibility, all or none."
                    );
                }
                let result = session.install_apk(p.path.clone()).await?;
                let mut said = vec![format!("Installed {}.", result.package)];
                let mut choices = Vec::new();
                for part in &result.parts {
                    match part.choice {
                        Some(on) => said.push(format!(
                            "{}, {}, is {}, as chosen before.",
                            part.name,
                            part.kind_description,
                            if on { "on" } else { "off" }
                        )),
                        None => {
                            let on = match policy.as_str() {
                                "all" => true,
                                "none" => false,
                                _ => matches!(part.kind, AppPartKind::AccessibilityService),
                            };
                            said.push(format!(
                                "{}, {}, is {}.",
                                part.name,
                                part.kind_description,
                                if on { "on" } else { "off" }
                            ));
                            choices.push(AppChoice {
                                part: part.clone(),
                                on,
                            });
                        }
                    }
                }
                if !choices.is_empty() {
                    session.set_app_choices(choices).await?;
                }
                text(said.join(" "))
            }
            .await,
        )
    }

    /// Lists a device's apps, by the names people see, with their packages.
    #[tool(annotations(read_only_hint = true))]
    async fn list_apps(
        &self,
        Parameters(p): Parameters<ListAppsParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let apps = session.list_apps(p.system.unwrap_or(false)).await?;
                json(
                    &apps
                        .into_iter()
                        .map(|a| App {
                            name: a.label,
                            package: a.package,
                            version: a.version,
                            system: a.system,
                            enabled: a.enabled,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            .await,
        )
    }

    /// Stops an app, as Force Stop in its settings does.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn force_stop_app(
        &self,
        Parameters(p): Parameters<AppParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let package = self.app_package(&session, &p.app).await?;
                session.force_stop_app(package.clone()).await?;
                text(format!("Stopped {package}."))
            }
            .await,
        )
    }

    /// Lists the permissions an app asks for, and its special access, such
    /// as display over other apps, with whether each is granted.
    #[tool(annotations(read_only_hint = true))]
    async fn app_permissions(
        &self,
        Parameters(p): Parameters<AppParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let package = self.app_package(&session, &p.app).await?;
                let found = session.app_permissions(package).await?;
                json(&Permissions {
                    permissions: found
                        .permissions
                        .into_iter()
                        .map(|p| Permission {
                            name: p.name,
                            description: p.label,
                            granted: p.granted,
                        })
                        .collect(),
                    special_access: found
                        .access
                        .into_iter()
                        .map(|a| Permission {
                            name: a.name.clone(),
                            description: a.name,
                            granted: a.allowed,
                        })
                        .collect(),
                })
            }
            .await,
        )
    }

    /// Grants or revokes one of an app's permissions, or grants them all.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn set_permission(
        &self,
        Parameters(p): Parameters<PermissionParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let package = self.app_package(&session, &p.app).await?;
                if p.permission.eq_ignore_ascii_case("all") {
                    if !p.granted {
                        anyhow::bail!("Revoke permissions one at a time.");
                    }
                    let granted = session.grant_all_permissions(package.clone()).await?;
                    return text(format!("Granted {granted} permissions to {package}."));
                }
                session
                    .set_app_permission(package.clone(), p.permission.clone(), p.granted)
                    .await?;
                text(format!(
                    "{} {} {} {package}.",
                    if p.granted { "Granted" } else { "Revoked" },
                    p.permission,
                    if p.granted { "to" } else { "from" }
                ))
            }
            .await,
        )
    }

    /// Lists a device's accessibility services, screen readers first, with
    /// whether each is on.
    #[tool(annotations(read_only_hint = true))]
    async fn list_services(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let services = session.list_services().await?;
                json(
                    &services
                        .into_iter()
                        .map(|s| Service {
                            name: s.label,
                            component: s.component,
                            description: s.description,
                            screen_reader: s.screen_reader,
                            on: s.on,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            .await,
        )
    }

    /// Turns an accessibility service on or off, and keeps it that way.
    /// Turning a screen reader on switches the device to it.
    #[tool(annotations(destructive_hint = false, idempotent_hint = true))]
    async fn set_service(
        &self,
        Parameters(p): Parameters<ServiceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let services = session.list_services().await?;
                let wanted = p.service.trim().to_lowercase();
                let service = services
                    .iter()
                    .find(|s| {
                        s.label.to_lowercase() == wanted || s.component.to_lowercase() == wanted
                    })
                    .or_else(|| {
                        services.iter().find(|s| {
                            s.component
                                .to_lowercase()
                                .starts_with(&format!("{wanted}/"))
                        })
                    })
                    .ok_or_else(|| {
                        let names: Vec<&str> = services.iter().map(|s| s.label.as_str()).collect();
                        anyhow::anyhow!(
                            "No accessibility service is called \"{}\". The device has: {}.",
                            p.service,
                            names.join(", ")
                        )
                    })?;
                session.set_service(service.component.clone(), p.on).await?;
                text(format!(
                    "{} is {}.",
                    service.label,
                    if p.on { "on" } else { "off" }
                ))
            }
            .await,
        )
    }

    /// Lists a device's snapshots, newest first, with their notes.
    #[tool(annotations(read_only_hint = true))]
    async fn list_snapshots(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let snapshots = session.snapshots().await?;
                json(
                    &snapshots
                        .into_iter()
                        .map(|s| Snapshot {
                            name: s.name,
                            notes: s.notes,
                            taken: s.taken,
                            size: s.size,
                            loaded: s.loaded,
                            usable: s.compatible,
                        })
                        .collect::<Vec<_>>(),
                )
            }
            .await,
        )
    }

    /// Saves the device as it is now, under a name, as a starting point to
    /// come back to with load_snapshot.
    #[tool(annotations(destructive_hint = false))]
    async fn save_snapshot(
        &self,
        Parameters(p): Parameters<SnapshotParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (device, session) = self.session(p.device.as_deref()).await?;
                session
                    .save_snapshot(p.name.clone(), p.notes.unwrap_or_default())
                    .await?;
                text(format!("Saved {} as \"{}\".", device.name, p.name))
            }
            .await,
        )
    }

    /// Puts the device back as it was in a snapshot. What's changed since
    /// is lost, unless it's saved first.
    #[tool(annotations(destructive_hint = true))]
    async fn load_snapshot(
        &self,
        Parameters(p): Parameters<SnapshotParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (device, session) = self.session(p.device.as_deref()).await?;
                let id = self.snapshot_id(&session, &p.name).await?;
                session.load_snapshot(id).await?;
                text(format!(
                    "{} is back as it was in \"{}\".",
                    device.name, p.name
                ))
            }
            .await,
        )
    }
}

#[tool_router(router = destructive_tools, vis = "pub(crate)")]
impl AaeServer {
    /// Wipes a device back to how it was first set up: its apps, data and
    /// snapshots are deleted, and its screen reader is set up again. It
    /// keeps its name, hardware and volume. Can't be undone.
    #[tool(annotations(destructive_hint = true))]
    async fn wipe_device(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let device = self.device(p.device.as_deref())?;
                self.forget(&device.id).await;
                let steps = Arc::new(Steps::default());
                self.engine
                    .wipe_device(device.id.clone(), steps.clone())
                    .await?;
                text(format!("Wiped {}. {}", device.name, steps.said()))
            }
            .await,
        )
    }

    /// Deletes a stopped device and all its files. Can't be undone.
    #[tool(annotations(destructive_hint = true))]
    async fn delete_device(
        &self,
        Parameters(p): Parameters<DeviceParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let Some(name) = p.device.as_deref() else {
                    anyhow::bail!("Name the device to delete, with `device`.");
                };
                let device = self.device(Some(name))?;
                if device.running {
                    anyhow::bail!("Stop {} before deleting it.", device.name);
                }
                self.forget(&device.id).await;
                let freed = self.engine.delete_device(device.id.clone())?;
                text(format!("Deleted {}. Freed {freed}.", device.name))
            }
            .await,
        )
    }

    /// Deletes one of a device's snapshots. Can't be undone.
    #[tool(annotations(destructive_hint = true))]
    async fn delete_snapshot(
        &self,
        Parameters(p): Parameters<SnapshotParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let id = self.snapshot_id(&session, &p.name).await?;
                session.delete_snapshot(id).await?;
                text(format!("Deleted the snapshot \"{}\".", p.name))
            }
            .await,
        )
    }

    /// Uninstalls an app, with its data. Can't be undone.
    #[tool(annotations(destructive_hint = true))]
    async fn uninstall_app(
        &self,
        Parameters(p): Parameters<AppParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let package = self.app_package(&session, &p.app).await?;
                session.uninstall_app(package.clone()).await?;
                text(format!("Uninstalled {package}."))
            }
            .await,
        )
    }

    /// Deletes an app's data, as if it had just been installed. Can't be undone.
    #[tool(annotations(destructive_hint = true))]
    async fn clear_app_data(
        &self,
        Parameters(p): Parameters<AppParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let package = self.app_package(&session, &p.app).await?;
                session.clear_app_data(package.clone()).await?;
                text(format!("Cleared {package}'s data."))
            }
            .await,
        )
    }

    /// Runs a command in the device's shell and returns everything it
    /// printed, with its exit status.
    #[tool(annotations(destructive_hint = true, open_world_hint = false))]
    async fn run_shell(
        &self,
        Parameters(p): Parameters<ShellParam>,
    ) -> Result<CallToolResult, ErrorData> {
        respond(
            async {
                let (_, session) = self.session(p.device.as_deref()).await?;
                let result = session.run_command(p.command).await?;
                text(format!(
                    "{}\n(exit status {})",
                    result.output.trim_end(),
                    result.status
                ))
            }
            .await,
        )
    }
}

impl AaeServer {
    async fn snapshot_id(
        &self,
        session: &Arc<aae_ffi::Session>,
        name: &str,
    ) -> anyhow::Result<String> {
        let snapshots = session.snapshots().await?;
        snapshots
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
            .map(|s| s.id.clone())
            .ok_or_else(|| {
                let names: Vec<&str> = snapshots.iter().map(|s| s.name.as_str()).collect();
                anyhow::anyhow!(
                    "There's no snapshot called \"{name}\". The snapshots are: {}.",
                    if names.is_empty() {
                        "none".into()
                    } else {
                        names.join(", ")
                    }
                )
            })
    }
}
