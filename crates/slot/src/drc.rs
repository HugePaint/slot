use crate::audio::Profile;

/// Widest correction the resampler is allowed to apply. The panel runs 60 Hz against the
/// GBA's 59.7275, a standing 0.456% surplus, so the bound has to sit just above that or the
/// buffer would climb with nothing able to pull it back.
const MAX_CORRECTION: f64 = 0.005;

pub fn drc_ratio(queued_frames: usize, target_frames: usize) -> f64 {
    if target_frames == 0 {
        return 1.0;
    }
    let error = target_frames as f64 - queued_frames as f64;
    let ratio = 1.0 + MAX_CORRECTION * error / target_frames as f64;
    ratio.clamp(1.0 - MAX_CORRECTION, 1.0 + MAX_CORRECTION)
}

/// Where the ring is primed, in frames: the `Stable` profile (`capacity / 2`).
///
/// A running session asks `Profile::target` instead, so Balanced and Strict park the ring
/// shallower than this without the ring itself having to know which profile opened the device.
pub fn drc_target(capacity_frames: usize) -> usize {
    Profile::Stable.target(capacity_frames)
}
