use slot_gfx::{Draw, TexId, OUT_H, OUT_W};
use slot_store::{BLUE_LIGHT_MAX, BRIGHTNESS_MAX, VOLUME_MAX};

use crate::icon::icon_box;
use crate::toast::toast_rect;
use crate::{Badge, Icon, Toast};

pub type Millis = u64;

pub const HUD_MS: Millis = 1500;
const FADE_MS: Millis = 250;
const PULSE_MS: Millis = 500;

pub const PLATE_H: f32 = 40.0;

/// Where the plate hangs when there is a letter band above it: flush against that band's lower
/// edge, so the dark starts exactly where the case stops.
///
/// The plate used to start hard at the top of the screen, which was right for a screen with
/// nothing at the top of it. On the shelf there is now a band there, with the index printed
/// along it and the clock and battery on its ends, and a plate over all of that for the second
/// and a half a level is up hides the very things the band is for. Under the band it covers
/// only the picture, which is what a plate is for.
///
/// It is the band's own height rather than a second number, so the two cannot drift apart.
pub const PLATE_Y: f32 = crate::slot_chrome::TOP_BAND_H;

pub const ALARM_BAND_H: f32 = 120.0;
const ALARM_BRIGHT: [f32; 4] = [0.82, 0.10, 0.10, 0.94];
const ALARM_DIM: [f32; 4] = [0.40, 0.03, 0.03, 0.94];

pub const HUD_ICON_PX: f32 = 24.0;
pub const HUD_INK: [u8; 3] = [0xf5, 0xf2, 0xef];
const ICON_GAP: f32 = 10.0;

const BADGE_MARGIN: f32 = 12.0;

pub fn badge_at(w: f32, h: f32) -> (f32, f32) {
    (OUT_W as f32 - BADGE_MARGIN - w, (PLATE_H - h) / 2.0)
}

const BAR_W: f32 = 320.0;
const BAR_H: f32 = 6.0;
const BAR_Y: f32 = (PLATE_H - BAR_H) / 2.0;
pub(crate) const TRACK: [f32; 4] = [1.0, 1.0, 1.0, 0.18];
pub(crate) const FILL: [f32; 4] = [
    HUD_INK[0] as f32 / 255.0,
    HUD_INK[1] as f32 / 255.0,
    HUD_INK[2] as f32 / 255.0,
    1.0,
];

/// The bar's fill. The palette's ink rather than a colour of its own, which is what the glyph
/// beside it is drawn in and what every letter on the shelf is set in: one ink per mode, and
/// the HUD is not a second one.
///
/// The track stays translucent white in both modes, and it is the one value in here that does
/// not flip. It is not type, it is the bar a fill runs along, and the plate under it is black
/// at 0.72 in the dark and would be the case's own light in the light — a track that matched
/// the plate would stop being a track.
fn fill() -> [f32; 4] {
    crate::palette::ink_f()
}

#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum HudKind {
    #[default]
    Brightness,
    BlueLight,
    Volume,
    Rewind,
}

impl HudKind {
    pub fn icon(self, value: u8, muted: bool, headphones: bool) -> Icon {
        match self {
            HudKind::Brightness => Icon::Brightness,
            HudKind::BlueLight => Icon::BlueLight,
            HudKind::Volume if headphones && muted => Icon::HeadphonesMuted,
            HudKind::Volume if headphones => Icon::Headphones,
            HudKind::Volume if muted => Icon::VolumeMuted,
            HudKind::Volume if value == 0 => Icon::VolumeZero,
            HudKind::Volume => Icon::Volume,
            HudKind::Rewind => Icon::Rewind,
        }
    }

    fn max(self) -> u8 {
        match self {
            HudKind::Brightness => BRIGHTNESS_MAX,
            HudKind::BlueLight => BLUE_LIGHT_MAX,
            HudKind::Volume => VOLUME_MAX,
            HudKind::Rewind => 100,
        }
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum FfState {
    #[default]
    Off,
    Held,
    Latched,
}

pub fn ff_badge(state: FfState) -> Option<Icon> {
    match state {
        FfState::Off => None,
        FfState::Held => Some(Icon::FastForward),
        FfState::Latched => Some(Icon::FastForwardLatched),
    }
}

pub const LINK_HOST_INK: [u8; 3] = [0x8a, 0x74, 0xcf];
pub const LINK_JOIN_INK: [u8; 3] = [0xb2, 0xb2, 0xb8];

#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub enum LinkBadge {
    #[default]
    Off,
    Hosting,
    Joined,
    HostingLost,
    JoinedLost,
}

impl LinkBadge {
    pub const FACES: [LinkBadge; 4] = [
        LinkBadge::Hosting,
        LinkBadge::Joined,
        LinkBadge::HostingLost,
        LinkBadge::JoinedLost,
    ];

    pub fn face_index(self) -> Option<usize> {
        LinkBadge::FACES.iter().position(|b| *b == self)
    }

    pub fn badge(self) -> Option<Badge> {
        match self {
            LinkBadge::Off => None,
            LinkBadge::Hosting | LinkBadge::Joined => Some(Badge::Link),
            LinkBadge::HostingLost | LinkBadge::JoinedLost => Some(Badge::LinkBroken),
        }
    }

    pub fn colour(self) -> Option<[u8; 3]> {
        match self {
            LinkBadge::Off => None,
            LinkBadge::Hosting | LinkBadge::HostingLost => Some(LINK_HOST_INK),
            LinkBadge::Joined | LinkBadge::JoinedLost => Some(LINK_JOIN_INK),
        }
    }
}

#[derive(Default)]
pub struct Hud {
    pub kind: HudKind,
    pub value: u8,
    pub shown_at: Option<Millis>,
    held: bool,
    muted: bool,
    headphones: bool,
    ff: FfState,
    said: Option<(Toast, Millis)>,
    alarm: Option<Millis>,
    icons: Vec<TexId>,
    toasts: Vec<TexId>,
    link: LinkBadge,
    link_faces: Vec<TexId>,
}

impl Hud {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_icons(&mut self, icons: Vec<TexId>) {
        self.icons = icons;
    }

    pub fn set_toasts(&mut self, toasts: Vec<TexId>) {
        self.toasts = toasts;
    }

    pub fn toast(&mut self, toast: Toast, now: Millis) {
        self.said = Some((toast, now));
    }

    pub fn set_alarm(&mut self, on: bool, now: Millis) {
        self.alarm = match (on, self.alarm) {
            (true, Some(since)) => Some(since),
            (true, None) => Some(now),
            (false, _) => None,
        };
    }

    pub fn alarmed(&self) -> bool {
        self.alarm.is_some()
    }

    pub fn toast_visible(&self, now: Millis) -> bool {
        self.toast_alpha(now) > 0.0
    }

    pub fn said(&self, now: Millis) -> Option<Toast> {
        self.toast_visible(now)
            .then(|| self.said.map(|(t, _)| t))
            .flatten()
    }

    pub fn show(&mut self, kind: HudKind, value: u8, muted: bool, now: Millis) {
        self.kind = kind;
        self.value = value;
        self.muted = muted;
        self.shown_at = Some(now);
        self.held = kind == HudKind::Rewind;
    }

    pub fn release_rewind(&mut self) {
        if self.kind == HudKind::Rewind {
            self.shown_at = None;
        }
        self.held = false;
    }

    pub fn glyph(&self) -> Icon {
        self.kind.icon(self.value, self.muted, self.headphones)
    }

    pub fn set_headphones(&mut self, on: bool) {
        self.headphones = on;
    }

    pub fn badge(&self) -> Option<Icon> {
        ff_badge(self.ff)
    }

    /// One glyph's face, whether or not the HUD is showing it. Keyed by the icon rather than by
    /// the kind, because the one caller that is not a level is the status row's mode badge:
    /// `Icon::of_mode` says which of the two it wants and the list is in `Icon::ALL` order.
    pub fn face(&self, icon: Icon) -> Option<TexId> {
        self.icons.get(icon.index()).copied()
    }

    /// Pushed from outside because the badge answers to the gesture machine's latch, which
    /// nothing on this side of the app can see.
    pub fn set_ff(&mut self, ff: FfState) {
        self.ff = ff;
    }

    pub fn set_link(&mut self, badge: LinkBadge) {
        self.link = badge;
    }

    pub fn link(&self) -> LinkBadge {
        self.link
    }

    pub fn set_link_faces(&mut self, faces: Vec<TexId>) {
        self.link_faces = faces;
    }

    pub fn visible(&self, now: Millis) -> bool {
        self.alpha(now) > 0.0
    }

    /// `top` is where the plate hangs, and it is the only number that has to be told: the bar,
    /// the toast and the badge are all laid out *inside* the plate, so moving the plate moves
    /// the whole readout. Callers pass `PLATE_Y` on a screen that has the letter band at the
    /// top and zero on one that does not.
    pub fn draw(&self, now: Millis, top: f32, out: &mut Vec<Draw>) {
        let alpha = self.alpha(now);
        let toast = self.toast_alpha(now);
        if alpha > 0.0 || toast > 0.0 {
            out.push(Draw::Rect {
                x: 0.0,
                y: top,
                w: OUT_W as f32,
                h: PLATE_H,
                colour: faded(crate::palette::plate(), alpha.max(toast)),
            });
            // They share the band, so only one at a time. A toast is a specific thing that just
            // happened and outranks a level the user can see the effect of anyway.
            if toast > 0.0 {
                self.draw_toast(top, now, out);
            } else {
                self.draw_bar(top, alpha, out);
            }
        }
        let link = self
            .link
            .face_index()
            .and_then(|i| self.link_faces.get(i).copied());
        let ff = ff_badge(self.ff).and_then(|i| self.icons.get(i.index()).copied());
        if let Some(tex) = link.or(ff) {
            self.place_badge(top, tex, out);
        }
        if let Some(since) = self.alarm {
            self.draw_alarm(now.saturating_sub(since), out);
        }
    }

    fn draw_bar(&self, top: f32, alpha: f32, out: &mut Vec<Draw>) {
        let x = (OUT_W as f32 - BAR_W) / 2.0;
        if let Some(tex) = self.icon() {
            let (w, h) = icon_box(HUD_ICON_PX);
            out.push(Draw::Tex {
                x: x - ICON_GAP - w as f32,
                y: top + (PLATE_H - h as f32) / 2.0,
                w: w as f32,
                h: h as f32,
                tex,
                alpha,
            });
        }
        out.push(Draw::Rect {
            x,
            y: top + BAR_Y,
            w: BAR_W,
            h: BAR_H,
            colour: faded(TRACK, alpha),
        });
        out.push(Draw::Rect {
            x,
            y: top + BAR_Y,
            w: BAR_W * self.fraction(),
            h: BAR_H,
            colour: faded(fill(), alpha),
        });
    }

    fn place_badge(&self, top: f32, tex: TexId, out: &mut Vec<Draw>) {
        let (w, h) = icon_box(HUD_ICON_PX);
        let (w, h) = (w as f32, h as f32);
        let (x, y) = badge_at(w, h);
        out.push(Draw::Tex {
            x,
            y: top + y,
            w,
            h,
            tex,
            alpha: 1.0,
        });
    }

    fn draw_alarm(&self, age: Millis, out: &mut Vec<Draw>) {
        out.push(Draw::Rect {
            x: 0.0,
            y: (OUT_H as f32 - ALARM_BAND_H) / 2.0,
            w: OUT_W as f32,
            h: ALARM_BAND_H,
            colour: pulse(age),
        });
        let alarm = Toast::CardUnwritable;
        let Some(tex) = self.toasts.get(alarm.index()).copied() else {
            return;
        };
        let (x, y, w, h) = toast_rect(alarm);
        out.push(Draw::Tex {
            x,
            y,
            w,
            h,
            tex,
            alpha: 1.0,
        });
    }

    /// No placeholder. A toast is type, and absent type is absent rather than a bar sitting
    /// where two words should be.
    fn draw_toast(&self, top: f32, now: Millis, out: &mut Vec<Draw>) {
        let alpha = self.toast_alpha(now);
        if alpha <= 0.0 {
            return;
        }
        let Some((said, _)) = self.said else {
            return;
        };
        let Some(tex) = self.toasts.get(said.index()).copied() else {
            return;
        };
        // `toast_rect` places a plate toast inside a plate standing at zero. An alarm is
        // already centred on the screen, so only the plate toast picks up `top`.
        let (x, mut y, w, h) = toast_rect(said);
        if !said.alarm() {
            y += top;
        }
        out.push(Draw::Tex {
            x,
            y,
            w,
            h,
            tex,
            alpha,
        });
    }

    fn icon(&self) -> Option<TexId> {
        self.icons.get(self.glyph().index()).copied()
    }

    fn fraction(&self) -> f32 {
        let max = self.kind.max();
        if max == 0 {
            return 0.0;
        }
        self.value.min(max) as f32 / max as f32
    }

    fn alpha(&self, now: Millis) -> f32 {
        let Some(shown) = self.shown_at else {
            return 0.0;
        };
        if self.held {
            return 1.0;
        }
        fade(shown, now)
    }

    fn toast_alpha(&self, now: Millis) -> f32 {
        match self.said {
            Some((_, shown)) => fade(shown, now),
            None => 0.0,
        }
    }
}

fn fade(shown: Millis, now: Millis) -> f32 {
    let age = now.saturating_sub(shown);
    if age >= HUD_MS {
        return 0.0;
    }
    ((HUD_MS - age) as f32 / FADE_MS as f32).min(1.0)
}

fn pulse(age: Millis) -> [f32; 4] {
    let phase = (age % PULSE_MS) as f32 / PULSE_MS as f32;
    let t = 0.5 + 0.5 * (phase * std::f32::consts::TAU).cos();
    std::array::from_fn(|i| ALARM_DIM[i] + (ALARM_BRIGHT[i] - ALARM_DIM[i]) * t)
}

fn faded(colour: [f32; 4], alpha: f32) -> [f32; 4] {
    [colour[0], colour[1], colour[2], colour[3] * alpha]
}
