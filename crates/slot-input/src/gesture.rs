use crate::{Btn, Millis, RawEvent};

pub const SELECT_CHORD_MS: Millis = 600;

/// How long the second key of the SELECT+Y chord is held before it stops being a tap and becomes
/// the palette browser. Long enough that walking the ring never opens a panel by accident, short
/// enough that a user who means it does not have to wait for it.
pub const COLOR_HOLD_MS: Millis = 550;

/// How long a delivered tap stays down. Press and release in one batch net out to nothing,
/// because the mask is set and cleared before the core reads it.
pub const SELECT_TAP_MS: Millis = 50;
pub const MENU_TAP_MS: Millis = 250;
pub const MENU_DOUBLE_TAP_MS: Millis = 350;
pub const MENU_HOLD_MS: Millis = 1000;
pub const FF_DOUBLE_TAP_MS: Millis = 250;
pub const MUTE_CHORD_MS: Millis = 200;
pub const POWER_HOLD_MS: Millis = 1000;

pub const VOLUME_REPEAT_DELAY_MS: Millis = 400;
pub const VOLUME_REPEAT_MS: Millis = 120;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Action {
    GbaDown(Btn),
    GbaUp(Btn),
    ShelfLeft,
    ShelfRight,
    Insert,
    Eject,
    Polaroids,
    SaveState,
    LoadState,
    RewindStart,
    RewindStop,
    FfStart,
    FfStop,
    BrightnessUp,
    BrightnessDown,
    BlueLightUp,
    BlueLightDown,
    VolumeUp,
    VolumeDown,
    QuickMenu,
    GameMenu,
    MuteToggle,
    ColourCorrectionToggle,
    PaletteNext,
    PalettePrev,
    ShaderNext,
    ShaderPrev,
    PowerPress,
    PowerTap,
    PowerHold,
    PowerOff,
    LidClose,
    LidOpen,
    /// SELECT+X, and what it steps depends on the machine: the **Overlay** art on a Game Boy and
    /// a Game Boy Color, and the aperture table on an Advance, the one machine that ships no art.
    /// `App`'s Phase::Playing is the fork — this only names the chord that reaches it. It is the
    /// **only** key that does it: `SELECT+R2` used to be a second one and is gone.
    MaskCycle,
    /// Cycle the in-game colour correction (GRAYSCALE / AGB-001 / NDS) on SELECT+Y,
    /// and the Game Boy's palette from the same key — a tap walks the page of palettes in hand.
    ColorCycle,
    /// The **hold** half of that same key, on the same press: SELECT+Y held opens the palette
    /// browser. Delivered at the threshold, while the key is still down, exactly as `PowerHold`
    /// is — the browser is on screen for as long as the user is holding it, and it is B (or the
    /// key's own release, which has by then been spent) that puts it away.
    ColorHold,
    /// Toggle the cart's cheat list on SELECT+A. The list lives on the card, so this only flips
    /// it on or off; what is in it is edited on a PC.
    CheatToggle,
    /// Step the audio profile (stable / balanced / strict) on SELECT+VOL+. Emitted on every
    /// screen, like the other chords: this file is blind to which one is up, and it is the app
    /// that decides where it lands. It lands on the shelf, because applying a profile reopens
    /// the PCM and only the shelf has nothing playing to interrupt.
    AudioProfileNext,
    /// The same the other way, on SELECT+VOL-.
    AudioProfilePrev,
    /// The shortcut card. The menu key now opens the quick menu; this variant stays so the
    /// about screen can still be opened directly, including from tests and the quick-menu row.
    OpenAbout,
    /// Which way round the frontend is printed, on SELECT+START.
    ///
    /// START rather than B, L2 or R2, the other three the chord table leaves free. START is
    /// what the shelf used to open the letter-view toggle with, before that toggle was removed,
    /// so it is the one free slot already carrying the meaning "how the shelf is shown" rather
    /// than being a key that has to be taught. And it costs nothing to reach for: a bare START
    /// on the shelf opens the core picker and a bare B does nothing, so neither half of this
    /// press is a gesture the user loses by making it a chord.
    ///
    /// Emitted on every screen, like the other chords — this file is blind to which one is up.
    /// It is the app that lands it on the shelf, where the index and the case are the things
    /// being printed.
    ModeToggle,
}

#[derive(Copy, Clone, Default)]
enum Select {
    #[default]
    Idle,
    Held {
        since: Millis,
        chorded: bool,
        sent: Option<Millis>,
    },
    ReleaseDue(Millis),
}

#[derive(Default)]
pub struct Gestures {
    select: Select,
    /// Buttons swallowed by a chord, so their release is swallowed too. `u16` rather than `u8`
    /// so the chord set can grow past eight buttons (each chord holds one bit).
    chord_held: u16,
    menu_down_at: Option<Millis>,
    menu_last_tap: Option<Millis>,
    menu_eject_fired: bool,
    power_down_at: Option<Millis>,
    power_hold_fired: bool,
    vol_up_at: Option<Millis>,
    vol_down_at: Option<Millis>,
    vol_up_ramp: Option<Millis>,
    vol_down_ramp: Option<Millis>,
    mute_fired: bool,
    ff_on: bool,
    ff_latched: bool,
    ff_latching_press: bool,
    r2_refused: bool,
    r2_last_release: Option<Millis>,
    rewinding: bool,
    /// SELECT+Y is the one chord that is two gestures on one press, so it is the one chord whose
    /// press is not spent on the way down: the Y is remembered here and the release decides. The
    /// hold is delivered from `tick`, while the key is still down, exactly as the POWER hold is.
    y_at: Option<Millis>,
    y_hold_fired: bool,
}

fn ramp_due(down: Option<Millis>, last: Option<Millis>, now: Millis) -> bool {
    let Some(down) = down else {
        return false;
    };
    if now.saturating_sub(down) < VOLUME_REPEAT_DELAY_MS {
        return false;
    }
    last.is_none_or(|l| now.saturating_sub(l) >= VOLUME_REPEAT_MS)
}

impl Gestures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ff_latched(&self) -> bool {
        self.ff_latched
    }

    pub fn drop_ff_latch(&mut self) -> Vec<Action> {
        if !self.ff_latched {
            return Vec::new();
        }
        self.ff_clear()
    }

    pub fn feed(&mut self, ev: RawEvent, now: Millis) -> Vec<Action> {
        match ev {
            RawEvent::Down(b) => self.down(b, now),
            RawEvent::Up(b) => self.up(b, now),
        }
    }

    pub fn tick(&mut self, now: Millis) -> Vec<Action> {
        let mut out = Vec::new();
        if let Select::ReleaseDue(due) = self.select {
            if now >= due {
                self.select = Select::Idle;
                out.push(Action::GbaUp(Btn::Select));
            }
        }
        if let Select::Held {
            since,
            chorded: false,
            sent: None,
        } = self.select
        {
            if now.saturating_sub(since) >= SELECT_CHORD_MS {
                out.extend(self.hand_over_select(now));
            }
        }
        if let Some(d) = self.menu_down_at {
            if !self.menu_eject_fired && now.saturating_sub(d) >= MENU_HOLD_MS {
                self.menu_eject_fired = true;
                out.push(Action::Eject);
            }
        }
        if let Some(d) = self.power_down_at {
            if !self.power_hold_fired && now.saturating_sub(d) >= POWER_HOLD_MS {
                self.power_hold_fired = true;
                out.push(Action::PowerHold);
            }
        }
        // The second half of SELECT+Y. Nothing is emitted on the way down, so this is the only
        // thing that can turn that press into a hold; a press that ends first is the walk instead.
        if let Some(d) = self.y_at {
            if !self.y_hold_fired && now.saturating_sub(d) >= COLOR_HOLD_MS {
                self.y_hold_fired = true;
                out.push(Action::ColorHold);
            }
        }
        // Held volume runs the level. Not while the pair has fired: that press was a mute,
        // and ramping under it would move the level the mute just remembered.
        if !self.mute_fired {
            if ramp_due(self.vol_up_at, self.vol_up_ramp, now) {
                self.vol_up_ramp = Some(now);
                out.push(Action::VolumeUp);
            }
            if ramp_due(self.vol_down_at, self.vol_down_ramp, now) {
                self.vol_down_ramp = Some(now);
                out.push(Action::VolumeDown);
            }
        }
        out
    }

    fn down(&mut self, b: Btn, now: Millis) -> Vec<Action> {
        if let (true, Some((bit, action))) = (self.chording(now), chord(b)) {
            self.mark_chorded();
            self.chord_held |= bit;
            // SELECT+Y is the one chord that is two gestures on one press. The hold is delivered
            // from `tick`; the tap is delivered from `up` when no hold got there first.
            if b == Btn::Y {
                self.y_at = Some(now);
                self.y_hold_fired = false;
                return Vec::new();
            }
            return vec![action];
        }
        match b {
            Btn::Select => self.select_down(now),
            Btn::Menu => self.menu_down(now),
            Btn::Power => {
                self.power_down_at = Some(now);
                self.power_hold_fired = false;
                vec![Action::PowerPress]
            }
            Btn::Lid => vec![Action::LidClose],
            Btn::VolUp | Btn::VolDown => self.volume_press(b, now),
            Btn::L2 => self.rewind_start(),
            Btn::R2 => self.ff_down(now),
            _ => {
                let mut out = self.hand_over_select(now);
                out.push(Action::GbaDown(b));
                out
            }
        }
    }

    fn up(&mut self, b: Btn, now: Millis) -> Vec<Action> {
        // The deferred half of SELECT+Y, and the only place a tap of it becomes a walk.
        // Guarded on the press having been ours: a bare Y is the core's and has to keep
        // its own release, and a bare Y on the shelf is what clears the machine hold.
        if b == Btn::Y && self.y_at.is_some() {
            if let Some((bit, _)) = chord(b) {
                self.chord_held &= !bit;
            }
            self.y_at = None;
            let held = std::mem::take(&mut self.y_hold_fired);
            // A hold has already been delivered, so the release owes nothing — the panel
            // it opened is still on screen and B is what puts it away.
            return if held {
                Vec::new()
            } else {
                vec![Action::ColorCycle]
            };
        }
        if let Some((bit, _)) = chord(b) {
            if self.chord_held & bit != 0 {
                self.chord_held &= !bit;
                return Vec::new();
            }
        }
        match b {
            Btn::Select => self.select_up(now),
            Btn::Menu => self.menu_up(now),
            Btn::Power => self.power_up(),
            Btn::Lid => vec![Action::LidOpen],
            Btn::VolUp | Btn::VolDown => self.volume_release(b),
            Btn::L2 => self.rewind_stop(),
            Btn::R2 => self.ff_up(now),
            _ => vec![Action::GbaUp(b)],
        }
    }

    fn chording(&self, now: Millis) -> bool {
        match self.select {
            Select::Held { since, chorded, .. } => {
                chorded || now.saturating_sub(since) < SELECT_CHORD_MS
            }
            _ => false,
        }
    }

    fn mark_chorded(&mut self) {
        if let Select::Held { chorded, .. } = &mut self.select {
            *chorded = true;
        }
    }

    fn select_down(&mut self, now: Millis) -> Vec<Action> {
        let mut out = Vec::new();
        if matches!(self.select, Select::ReleaseDue(_)) {
            out.push(Action::GbaUp(Btn::Select));
        }
        if !matches!(self.select, Select::Held { .. }) {
            self.select = Select::Held {
                since: now,
                chorded: false,
                sent: None,
            };
        }
        out
    }

    fn hand_over_select(&mut self, now: Millis) -> Vec<Action> {
        match &mut self.select {
            Select::Held {
                chorded: false,
                sent: sent @ None,
                ..
            } => {
                *sent = Some(now);
                vec![Action::GbaDown(Btn::Select)]
            }
            _ => Vec::new(),
        }
    }

    fn select_up(&mut self, now: Millis) -> Vec<Action> {
        let Select::Held { chorded, sent, .. } = std::mem::take(&mut self.select) else {
            return Vec::new();
        };
        match (sent, chorded) {
            (Some(at), _) if now.saturating_sub(at) >= SELECT_TAP_MS => {
                vec![Action::GbaUp(Btn::Select)]
            }
            (Some(at), _) => {
                self.select = Select::ReleaseDue(at + SELECT_TAP_MS);
                Vec::new()
            }
            (None, true) => Vec::new(),
            (None, false) => {
                self.select = Select::ReleaseDue(now + SELECT_TAP_MS);
                vec![Action::GbaDown(Btn::Select)]
            }
        }
    }

    fn menu_down(&mut self, now: Millis) -> Vec<Action> {
        if self.chording(now) {
            self.mark_chorded();
            self.menu_down_at = None;
            self.menu_last_tap = None;
            return vec![Action::GameMenu];
        }
        if let Some(tap) = self.menu_last_tap {
            if now.saturating_sub(tap) <= MENU_DOUBLE_TAP_MS {
                self.menu_last_tap = None;
                self.menu_down_at = None;
                return vec![Action::Polaroids];
            }
        }
        self.menu_down_at = Some(now);
        self.menu_eject_fired = false;
        Vec::new()
    }

    fn menu_up(&mut self, now: Millis) -> Vec<Action> {
        let Some(d) = self.menu_down_at.take() else {
            return Vec::new();
        };
        let ejected = self.menu_eject_fired;
        self.menu_eject_fired = false;
        let tapped = !ejected && now.saturating_sub(d) < MENU_TAP_MS;
        self.menu_last_tap = tapped.then_some(now);
        match tapped {
            true => vec![Action::QuickMenu],
            false => Vec::new(),
        }
    }

    fn power_up(&mut self) -> Vec<Action> {
        let held = self.power_hold_fired;
        self.power_down_at = None;
        self.power_hold_fired = false;
        vec![if held {
            Action::PowerOff
        } else {
            Action::PowerTap
        }]
    }

    fn volume_press(&mut self, b: Btn, now: Millis) -> Vec<Action> {
        let (mine, other, action) = match b {
            Btn::VolUp => (&mut self.vol_up_at, self.vol_down_at, Action::VolumeUp),
            _ => (&mut self.vol_down_at, self.vol_up_at, Action::VolumeDown),
        };
        *mine = Some(now);
        match b {
            Btn::VolUp => self.vol_up_ramp = None,
            _ => self.vol_down_ramp = None,
        }
        let mut out = vec![action];
        let paired = other.is_some_and(|t| now.abs_diff(t) <= MUTE_CHORD_MS);
        if paired && !self.mute_fired {
            self.mute_fired = true;
            out.push(Action::MuteToggle);
        }
        out
    }

    fn volume_release(&mut self, b: Btn) -> Vec<Action> {
        match b {
            Btn::VolUp => (self.vol_up_at, self.vol_up_ramp) = (None, None),
            _ => (self.vol_down_at, self.vol_down_ramp) = (None, None),
        }
        if self.vol_up_at.is_none() && self.vol_down_at.is_none() {
            self.mute_fired = false;
        }
        Vec::new()
    }

    fn rewind_start(&mut self) -> Vec<Action> {
        if self.rewinding {
            return Vec::new();
        }
        let mut out = Vec::new();
        out.extend(self.ff_clear());
        self.rewinding = true;
        out.push(Action::RewindStart);
        out
    }

    fn rewind_stop(&mut self) -> Vec<Action> {
        if !self.rewinding {
            return Vec::new();
        }
        self.rewinding = false;
        vec![Action::RewindStop]
    }

    fn ff_down(&mut self, now: Millis) -> Vec<Action> {
        if self.rewinding {
            self.r2_refused = true;
            return Vec::new();
        }
        if self.ff_latched {
            self.ff_latching_press = false;
        } else {
            let double = self
                .r2_last_release
                .is_some_and(|rel| now.saturating_sub(rel) <= FF_DOUBLE_TAP_MS);
            self.ff_latched = double;
            self.ff_latching_press = double;
        }
        if self.ff_on {
            return Vec::new();
        }
        self.ff_on = true;
        vec![Action::FfStart]
    }

    fn ff_up(&mut self, now: Millis) -> Vec<Action> {
        if std::mem::take(&mut self.r2_refused) {
            return Vec::new();
        }
        self.r2_last_release = Some(now);
        if self.ff_latching_press {
            self.ff_latching_press = false;
            return Vec::new();
        }
        self.ff_clear()
    }

    fn ff_clear(&mut self) -> Vec<Action> {
        self.ff_latched = false;
        self.ff_latching_press = false;
        if !self.ff_on {
            return Vec::new();
        }
        self.ff_on = false;
        vec![Action::FfStop]
    }
}

fn chord(b: Btn) -> Option<(u16, Action)> {
    Some(match b {
        Btn::Up => (1, Action::BrightnessUp),
        Btn::Down => (2, Action::BrightnessDown),
        Btn::Left => (4, Action::BlueLightDown),
        Btn::Right => (8, Action::BlueLightUp),
        Btn::L1 => (16, Action::LoadState),
        Btn::R1 => (32, Action::SaveState),
        Btn::X => (64, Action::MaskCycle),
        Btn::Y => (128, Action::ColorCycle),
        Btn::A => (256, Action::CheatToggle),
        Btn::Start => (512, Action::ModeToggle),
        Btn::L2 => (1024, Action::PalettePrev),
        Btn::R2 => (2048, Action::PaletteNext),
        // SELECT+either volume key mutes. A 34XX volume wheel cannot press VOL+ and VOL-
        // together, so the pair chord below never happens there.
        Btn::VolUp => (4096, Action::MuteToggle),
        Btn::VolDown => (8192, Action::MuteToggle),
        _ => return None,
    })
}
