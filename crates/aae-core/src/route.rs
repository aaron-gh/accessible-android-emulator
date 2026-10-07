//! Playing a route from a GPX file: the device's location moves along it,
//! once a second, at the pace its times give, or at a steady speed when it
//! has none, as a route planned rather than travelled doesn't.

use std::time::Duration;

use crate::control::Controller;
use crate::error::{Error, Result};

/// One point of a route.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres above sea level, if given.
    pub altitude: Option<f64>,
    /// Seconds since 1970, if given.
    pub time: Option<f64>,
}

/// The speed used for routes without times, in metres a second: 30 km/h.
pub const UNTIMED_SPEED: f64 = 30.0 / 3.6;

/// Reads a GPX file's track points, or else its route points, or else its
/// waypoints, in order.
pub fn parse(text: &str) -> Result<Vec<Point>> {
    let doc = roxmltree::Document::parse(text)
        .map_err(|e| Error::Message(format!("That isn't a GPX file AAE can read: {e}")))?;
    let points = |tag: &str| -> Vec<Point> {
        doc.descendants()
            .filter(|n| n.tag_name().name() == tag)
            .filter_map(|n| {
                let latitude = n.attribute("lat")?.trim().parse().ok()?;
                let longitude = n.attribute("lon")?.trim().parse().ok()?;
                let child = |name: &str| {
                    n.children()
                        .find(|c| c.tag_name().name() == name)
                        .and_then(|c| c.text())
                        .map(str::trim)
                };
                Some(Point {
                    latitude,
                    longitude,
                    altitude: child("ele").and_then(|e| e.parse().ok()),
                    time: child("time").and_then(parse_time),
                })
            })
            .collect()
    };
    let found = ["trkpt", "rtept", "wpt"]
        .iter()
        .map(|tag| points(tag))
        .find(|p| !p.is_empty())
        .unwrap_or_default();
    if found.is_empty() {
        return Err(Error::Message(
            "The GPX file has no points: no track, route or waypoints.".into(),
        ));
    }
    Ok(found)
}

/// An ISO 8601 time, such as "2024-05-01T09:30:00Z" or with fractions of a
/// second or an offset, as seconds since 1970.
fn parse_time(text: &str) -> Option<f64> {
    let (date, rest) = text.split_once('T')?;
    let mut date = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    // The offset, if any, after the seconds.
    let split = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(split);
    let mut clock = clock.split(':');
    let h: f64 = clock.next()?.parse().ok()?;
    let min: f64 = clock.next()?.parse().ok()?;
    let sec: f64 = clock.next().unwrap_or("0").parse().ok()?;
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let mut parts = zone[1..].split(':');
            let oh: f64 = parts.next()?.parse().ok()?;
            let om: f64 = parts.next().unwrap_or("0").parse().ok()?;
            let seconds = oh * 3600.0 + om * 60.0;
            if sign == '+' { seconds } else { -seconds }
        }
        _ => 0.0,
    };
    // Days since 1970 for a date, after Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days as f64 * 86_400.0 + h * 3600.0 + min * 60.0 + sec - offset)
}

/// Metres between two points, along the earth's surface.
pub fn distance(a: &Point, b: &Point) -> f64 {
    let r = 6_371_000.0;
    let (la1, la2) = (a.latitude.to_radians(), b.latitude.to_radians());
    let dla = la2 - la1;
    let dlo = (b.longitude - a.longitude).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * r * h.sqrt().asin()
}

/// The heading from one point to the next, in degrees from north.
fn bearing(a: &Point, b: &Point) -> f64 {
    let (la1, la2) = (a.latitude.to_radians(), b.latitude.to_radians());
    let dlo = (b.longitude - a.longitude).to_radians();
    let y = dlo.sin() * la2.cos();
    let x = la1.cos() * la2.sin() - la1.sin() * la2.cos() * dlo.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// How long the route takes at `speed` times its own pace, in seconds.
pub fn duration(points: &[Point], speed: f64) -> f64 {
    points
        .windows(2)
        .map(|w| leg_seconds(&w[0], &w[1]) / speed)
        .sum()
}

/// How long one leg takes at the route's own pace.
fn leg_seconds(a: &Point, b: &Point) -> f64 {
    match (a.time, b.time) {
        (Some(t1), Some(t2)) if t2 > t1 => t2 - t1,
        _ => distance(a, b) / UNTIMED_SPEED,
    }
}

/// Moves the device along the route, at `speed` times its own pace, updating
/// its location once a second, as a phone's GPS does. Returns at the end.
pub async fn play(controller: &Controller, points: &[Point], speed: f64) -> Result<()> {
    let speed = speed.clamp(0.1, 100.0);
    let first = points
        .first()
        .ok_or_else(|| Error::Message("The route has no points.".into()))?;
    let at = |p: &Point, metres_a_second: f64, heading: f64| {
        controller.set_moving_location(
            p.latitude,
            p.longitude,
            p.altitude.unwrap_or(10.0),
            metres_a_second,
            heading,
        )
    };
    at(first, 0.0, 0.0).await?;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.tick().await;
    for leg in points.windows(2) {
        let (a, b) = (&leg[0], &leg[1]);
        let seconds = leg_seconds(a, b) / speed;
        let metres_a_second = if seconds > 0.0 {
            distance(a, b) / seconds
        } else {
            0.0
        };
        let heading = bearing(a, b);
        // Each whole second of the leg, then its end.
        let steps = seconds.floor().max(0.0) as u64;
        for step in 1..=steps {
            let f = step as f64 / seconds;
            let here = Point {
                latitude: a.latitude + (b.latitude - a.latitude) * f,
                longitude: a.longitude + (b.longitude - a.longitude) * f,
                altitude: match (a.altitude, b.altitude) {
                    (Some(x), Some(y)) => Some(x + (y - x) * f),
                    (x, y) => x.or(y),
                },
                time: None,
            };
            tick.tick().await;
            at(&here, metres_a_second, heading).await?;
        }
        if seconds.fract() > 0.05 || steps == 0 {
            tokio::time::sleep(Duration::from_secs_f64(seconds.fract())).await;
            at(b, metres_a_second, heading).await?;
            tick.reset();
        }
    }
    let last = points.last().unwrap_or(first);
    at(last, 0.0, 0.0).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = r#"<?xml version="1.0"?>
<gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1">
  <trk><trkseg>
    <trkpt lat="51.5000" lon="-0.1200"><ele>12</ele><time>2024-05-01T09:30:00Z</time></trkpt>
    <trkpt lat="51.5010" lon="-0.1200"><time>2024-05-01T09:30:20Z</time></trkpt>
    <trkpt lat="51.5010" lon="-0.1180"><time>2024-05-01T10:30:40+01:00</time></trkpt>
  </trkseg></trk>
</gpx>"#;

    #[test]
    fn tracks_are_read_with_their_times() {
        let points = parse(TRACK).unwrap();
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].altitude, Some(12.0));
        assert_eq!(points[0].time, Some(1_714_555_800.0));
        // The offset is taken off: 10:30:40+01:00 is 09:30:40 UTC.
        assert_eq!(points[2].time, Some(1_714_555_840.0));
        assert_eq!(duration(&points, 1.0), 40.0);
        assert_eq!(duration(&points, 2.0), 20.0);
    }

    #[test]
    fn routes_without_times_go_at_a_steady_speed() {
        let gpx = r#"<gpx><rte><rtept lat="0" lon="0"/><rtept lat="0" lon="0.01"/></rte></gpx>"#;
        let points = parse(gpx).unwrap();
        // About 1113 metres at 30 km/h.
        let seconds = duration(&points, 1.0);
        assert!((seconds - 1113.2 / UNTIMED_SPEED).abs() < 2.0, "{seconds}");
    }

    #[test]
    fn a_file_without_points_says_so() {
        assert!(parse("<gpx></gpx>").is_err());
        assert!(parse("not xml").is_err());
    }
}
