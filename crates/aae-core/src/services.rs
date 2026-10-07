//! A device's accessibility services: which are installed and on, turning
//! any of them on or off, and switching screen readers. Names and
//! descriptions come from AAE's helper, as Android's settings show them.

use serde::Deserialize;

use crate::adb::{Adb, same_component};
use crate::device::Device;
use crate::error::{Error, Result};
use crate::provision::SERVICE_TIMEOUT;

const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const LIST_SERVICES: &str = "io.github.aaron_gh.aae.helper.LIST_SERVICES";

/// An installed accessibility service.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// As `package/class`.
    pub component: String,
    /// The name people see, such as "TalkBack".
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// It explores the screen by touch and speaks, as TalkBack does.
    #[serde(default)]
    pub screen_reader: bool,
    /// Turned on in Android's settings.
    #[serde(skip)]
    pub on: bool,
    /// The device's screen reader, as AAE keeps it.
    #[serde(skip)]
    pub current_screen_reader: bool,
}

impl Service {
    pub fn package(&self) -> &str {
        self.component.split('/').next().unwrap_or(&self.component)
    }
}

/// Every installed accessibility service, screen readers first, then by name.
pub async fn list(adb: &Adb, device: &Device) -> Result<Vec<Service>> {
    let out = adb
        .shell(&format!(
            "am broadcast -n {HELPER_RECEIVER} -a {LIST_SERVICES}"
        ))
        .await?;
    if !out.contains("result=1") {
        return Err(Error::Adb(
            "AAE's helper didn't answer. Starting the device again updates it.".into(),
        ));
    }
    let json = out
        .split_once("data=\"")
        .and_then(|(_, rest)| rest.rsplit_once('"'))
        .map(|(json, _)| json)
        .ok_or_else(|| Error::Adb("AAE's helper sent no answer.".into()))?;
    let mut services: Vec<Service> = serde_json::from_str(json)
        .map_err(|e| Error::Adb(format!("AAE's helper sent a list AAE can't read: {e}")))?;
    let enabled = adb.enabled_services().await?;
    for service in &mut services {
        service.on = enabled
            .iter()
            .any(|c| same_component(c, &service.component));
        service.current_screen_reader = device
            .meta
            .screen_reader
            .as_deref()
            .is_some_and(|c| same_component(c, &service.component));
    }
    services.sort_by_key(|s| (!s.screen_reader, s.label.to_lowercase()));
    Ok(services)
}

/// Finds a service by its name, package or component, ignoring case.
pub fn find<'a>(services: &'a [Service], name: &str) -> Option<&'a Service> {
    let name = name.trim();
    services
        .iter()
        .find(|s| same_component(&s.component, name))
        .or_else(|| services.iter().find(|s| s.label.eq_ignore_ascii_case(name)))
        .or_else(|| {
            services
                .iter()
                .find(|s| s.package().eq_ignore_ascii_case(name))
        })
}

/// Turns a service on, to stay on, or off, to stay off. Turning on a
/// screen reader makes it the device's screen reader instead of the one it
/// had, as two can't run at once. Turning off the device's screen reader
/// leaves it with none, and AAE won't offer one.
pub async fn set(device: &mut Device, adb: &Adb, service: &Service, on: bool) -> Result<()> {
    if on && service.screen_reader {
        return use_screen_reader(device, adb, service).await;
    }
    let component = service.component.clone();
    device.meta.app_choices.insert(component.clone(), on);
    if on {
        device.keep_service_enabled(&component);
        device.save_meta()?;
        adb.ensure_services(std::slice::from_ref(&component), SERVICE_TIMEOUT)
            .await?;
    } else {
        device
            .meta
            .keep_enabled
            .retain(|c| !same_component(c, &component));
        if device
            .meta
            .screen_reader
            .as_deref()
            .is_some_and(|c| same_component(c, &component))
        {
            device.meta.screen_reader = None;
            device.meta.screen_reader_declined = true;
        }
        device.save_meta()?;
        adb.disable_service(&component).await?;
    }
    Ok(())
}

/// Makes an installed screen reader the device's screen reader, turning off
/// any other, and keeps it on.
pub async fn use_screen_reader(device: &mut Device, adb: &Adb, service: &Service) -> Result<()> {
    let component = service.component.clone();
    // Every other screen reader goes off, the old one included.
    for other in list(adb, device).await? {
        if other.screen_reader && other.on && !same_component(&other.component, &component) {
            device
                .meta
                .keep_enabled
                .retain(|c| !same_component(c, &other.component));
            device
                .meta
                .app_choices
                .insert(other.component.clone(), false);
            adb.disable_service(&other.component).await?;
        }
    }
    if let Some(old) = device.meta.screen_reader.take() {
        device
            .meta
            .keep_enabled
            .retain(|c| !same_component(c, &old));
    }
    device.meta.screen_reader = Some(component.clone());
    device.meta.screen_reader_declined = false;
    device.meta.app_choices.insert(component.clone(), true);
    device.keep_service_enabled(&component);
    device.save_meta()?;
    adb.ensure_services(std::slice::from_ref(&component), SERVICE_TIMEOUT)
        .await?;
    Ok(())
}
