use crate::surface::{OUT_H, OUT_W};

const LINE_PX: f32 = 2.0;

const DOT_T: f32 = 0.12;

const OVERSHOOT: f32 = 0.6;

pub fn screen_scale(t: f32) -> f32 {
    let left = 1.0 - t.clamp(0.0, 1.0);
    1.0 - (1.0 - LINE_PX / OUT_H as f32) * left * left
}

pub fn screen_width(t: f32) -> f32 {
    let left = 1.0 - (t.clamp(0.0, 1.0) / DOT_T).min(1.0);
    1.0 - (1.0 - LINE_PX / OUT_W as f32) * left
}

pub fn screen_brightness(t: f32) -> f32 {
    let left = 1.0 - t.clamp(0.0, 1.0);
    1.0 + OVERSHOOT * left * left
}

/// The rect the game layer fills, in offscreen pixels. Centred: the line is at the vertical
/// middle of the frame, not at the slot.
///
/// The GBA's window, which is the whole frame — see `screen_rect_in` for a picture whose
/// window is smaller than the frame.
pub fn screen_rect(t: f32) -> (f32, f32, f32, f32) {
    screen_rect_in((0.0, 0.0, OUT_W as f32, OUT_H as f32), t)
}

/// The same strike and bloom inside a window of its own, for a machine whose picture does
/// not fill the frame.
///
/// Everything above is written against the whole frame, and the Game Boy's window is only
/// part of it: a picture that opened from the middle of the panel and settled into a window
/// beside it would read as the panel being *oddly* switched on, rather than as a smaller
/// screen coming up. So the collapse is applied to the window, centred on the window, and
/// the GBA remains the case where the window happens to be everything.
pub fn screen_rect_in(base: (f32, f32, f32, f32), t: f32) -> (f32, f32, f32, f32) {
    let (bx, by, bw, bh) = base;
    let w = bw * screen_width(t);
    let h = bh * screen_scale(t);
    (bx + (bw - w) / 2.0, by + (bh - h) / 2.0, w, h)
}
