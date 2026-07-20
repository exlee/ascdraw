use std::time::{Duration, Instant};

use super::RasterRefreshThrottle;

#[test]
fn metric_refreshes_are_limited_to_once_per_interval() {
    let start = Instant::now();
    let mut throttle = RasterRefreshThrottle::default();

    assert!(throttle.use_current_metrics(1, 10, start));
    assert!(!throttle.use_current_metrics(1, 11, start + Duration::from_millis(10)));
    assert_eq!(throttle.deadline(), Some(start + Duration::from_millis(50)));
    assert!(!throttle.promote_if_due(start + Duration::from_millis(49)));
    assert!(throttle.promote_if_due(start + Duration::from_millis(50)));
    assert!(throttle.use_current_metrics(1, 11, start + Duration::from_millis(50)));
}

#[test]
fn pending_refresh_tracks_latest_zoom_and_style_changes_are_immediate() {
    let start = Instant::now();
    let mut throttle = RasterRefreshThrottle::default();

    assert!(throttle.use_current_metrics(1, 10, start));
    assert!(!throttle.use_current_metrics(1, 11, start + Duration::from_millis(10)));
    assert!(!throttle.use_current_metrics(1, 12, start + Duration::from_millis(20)));
    assert!(throttle.promote_if_due(start + Duration::from_millis(50)));
    assert!(throttle.use_current_metrics(1, 12, start + Duration::from_millis(50)));
    assert!(throttle.use_current_metrics(2, 12, start + Duration::from_millis(51)));
}
