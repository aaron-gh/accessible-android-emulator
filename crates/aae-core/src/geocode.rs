//! Finding where a place is, to set a device's location from an address or
//! a place's name. The Mac app uses Apple's geocoder; elsewhere AAE asks
//! OpenStreetMap's Nominatim, which only gets the text the user typed.

use serde::Deserialize;

use crate::error::{Error, Result};

const SEARCH: &str = "https://nominatim.openstreetmap.org/search";

/// A place found, with where it is.
#[derive(Debug, Clone)]
pub struct Place {
    pub latitude: f64,
    pub longitude: f64,
    /// Such as "Eiffel Tower, Paris, France".
    pub name: String,
}

#[derive(Deserialize)]
struct Hit {
    lat: String,
    lon: String,
    display_name: String,
}

/// Latitude and longitude typed as "48.858, 2.294" or "48.858 2.294".
pub fn coordinates(text: &str) -> Option<(f64, f64)> {
    let numbers: Vec<f64> = text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    match numbers.as_slice() {
        [lat, lon] if lat.abs() <= 90.0 && lon.abs() <= 180.0 => Some((*lat, *lon)),
        _ => None,
    }
}

/// Where a place or address is, or none if nothing matches. Blocks.
pub fn look_up(text: &str) -> Result<Option<Place>> {
    let failed = |e: String| Error::Download(format!("Couldn't look up the place: {e}"));
    let body = ureq::get(SEARCH)
        .query("q", text)
        .query("format", "jsonv2")
        .query("limit", "1")
        // Nominatim asks every program to say who it is.
        .header(
            "User-Agent",
            "accessible-android-emulator (https://github.com/aaron-gh/accessible-android-emulator)",
        )
        .call()
        .map_err(|e| failed(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| failed(e.to_string()))?;
    let hits: Vec<Hit> = serde_json::from_str(&body).map_err(|e| failed(e.to_string()))?;
    Ok(hits.into_iter().next().and_then(|hit| {
        Some(Place {
            latitude: hit.lat.parse().ok()?,
            longitude: hit.lon.parse().ok()?,
            name: hit.display_name,
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_typed_coordinates() {
        assert_eq!(coordinates("48.858, 2.294"), Some((48.858, 2.294)));
        assert_eq!(coordinates("-33.86 151.21"), Some((-33.86, 151.21)));
        assert_eq!(coordinates("Paris"), None);
        assert_eq!(coordinates("100, 0"), None);
    }

    /// Asks OpenStreetMap, so only on request: cargo test -p aae-core geocode -- --ignored
    #[test]
    #[ignore]
    fn finds_a_place() {
        let place = look_up("Eiffel Tower, Paris").unwrap().unwrap();
        assert!((place.latitude - 48.858).abs() < 0.01, "{place:?}");
        assert!((place.longitude - 2.294).abs() < 0.01, "{place:?}");
    }
}
