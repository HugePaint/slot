use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::atomic::atomic_write;
use crate::gb_palette::GbPalette;
use crate::platform::Platform;

/// Twenty steps, and so nineteen of them lit: step 0 is the panel off, which is what the lid
/// closes with. Ten was too coarse at the bottom — step 1 sat well above what a dark room
/// wants, with nothing between it and black — so the ramp is finer rather than the top higher.
pub const BRIGHTNESS_MAX: u8 = 19;
pub const BLUE_LIGHT_MAX: u8 = 9;
pub const VOLUME_MAX: u8 = 100;

pub const UTC_OFFSET_MIN: i16 = -720;
pub const UTC_OFFSET_MAX: i16 = 840;

/// Which way round the frontend prints itself: light type on dark plastic, or dark type on
/// light plastic.
///
/// It lives here rather than in the ui because it is a setting the card remembers, like the
/// brightness and the volume, and the ui reads its palette off whatever is in the state file.
/// A `Mode` owned by the drawing code would have to be mirrored here anyway, and two enums for
/// one line of text is how a file and a screen come apart.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

impl Mode {
    /// The other one. What SELECT+START does, and the whole of what it does: there is no third
    /// mode to step through, so a toggle is the honest shape of the control.
    pub fn other(self) -> Mode {
        match self {
            Mode::Dark => Mode::Light,
            Mode::Light => Mode::Dark,
        }
    }

    /// What the state file calls it. Spelled at the definition rather than at either end, so
    /// the reader and the writer cannot disagree about the word.
    pub fn word(self) -> &'static str {
        match self {
            Mode::Dark => "dark",
            Mode::Light => "light",
        }
    }

    /// Anything else is not a mode. A state file edited on a desktop must not be able to put
    /// the device into a third one.
    pub fn from_word(word: &str) -> Option<Mode> {
        match word.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Mode::Dark),
            "light" => Some(Mode::Light),
            _ => None,
        }
    }
}

pub const FF_SPEEDS: [u8; 4] = [2, 3, 4, 6];

pub const FF_SPEED_DEFAULT: u8 = 6;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SlotState {
    pub cart: Option<String>,
    pub cart_platform: Option<Platform>,
    pub brightness: u8,
    pub blue_light: u8,
    pub volume: u8,
    pub volume_hp: u8,
    pub muted: bool,
    pub muted_hp: bool,
    pub clock_set: bool,
    pub utc_offset_min: i16,
    /// Light or dark. The one field whose absence is not an error — see `parse`.
    pub mode: Mode,
    /// The carts the user has starred, as filename stems. A `BTreeSet` rather than a `Vec`
    /// because it is a membership question asked once per cart per frame on the shelf, and
    /// because the order is never read: the shelf is ordered by the library, not by when a
    /// star was given. Written one `favorites=` line per stem, so a stem carrying `=`, a space
    /// or a comma survives the round trip (see `parse`).
    pub favorites: BTreeSet<String>,
    pub rumble: bool,
    pub ff_speed: u8,
    pub ff_sound: bool,
    pub colour_correction: bool,
    pub shader_gba: Shader,
    pub shader_gb: Shader,
    pub eject_save: bool,
    pub turbo: bool,
    pub rewind: bool,
    pub gb_palettes: bool,
    pub gb_palette: GbPalette,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Shader {
    Off,
    Lcd3x,
    Grid,
    Dot,
    Simpletex,
}

impl Shader {
    pub const GBA: [Shader; 4] = [Shader::Off, Shader::Lcd3x, Shader::Grid, Shader::Dot];
    pub const GB: [Shader; 3] = [Shader::Off, Shader::Grid, Shader::Simpletex];

    pub fn name(self) -> &'static str {
        match self {
            Shader::Off => "off",
            Shader::Lcd3x => "lcd3x",
            Shader::Grid => "grid",
            Shader::Dot => "dot",
            Shader::Simpletex => "simpletex",
        }
    }

    pub fn parse(name: &str) -> Option<Shader> {
        [
            Shader::Off,
            Shader::Lcd3x,
            Shader::Grid,
            Shader::Dot,
            Shader::Simpletex,
        ]
        .into_iter()
        .find(|s| s.name() == name)
    }

    pub fn step(self, choices: &[Shader], right: bool) -> Shader {
        let n = choices.len();
        let at = choices.iter().position(|&s| s == self).unwrap_or(0);
        choices[if right {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        }]
    }
}

/// Not derived. `read_slot_state` falls back here on a first boot, and all zeroes would
/// be a device with the backlight off and the mixer muted. The brightness default is the
/// middle of the ramp about to be written, not five of it: on a twenty step scale the old
/// five would come up at a quarter, and a card that has never been touched should look like
/// a card someone set at a sensible level.
impl Default for SlotState {
    fn default() -> Self {
        SlotState {
            cart: None,
            cart_platform: None,
            brightness: 10,
            blue_light: 0,
            volume: 60,
            volume_hp: 60,
            muted: false,
            muted_hp: false,
            clock_set: false,
            utc_offset_min: 0,
            mode: Mode::Dark,
            favorites: BTreeSet::new(),
            rumble: true,
            ff_speed: FF_SPEED_DEFAULT,
            ff_sound: false,
            colour_correction: false,
            shader_gba: Shader::Grid,
            shader_gb: Shader::Simpletex,
            eject_save: true,
            turbo: true,
            rewind: true,
            gb_palettes: false,
            gb_palette: GbPalette::DEFAULT,
        }
    }
}

fn state_path(root: &Path) -> PathBuf {
    root.join("Config").join("slot.state")
}

pub fn read_slot_state(root: &Path) -> SlotState {
    std::fs::read(state_path(root))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| parse(&s))
        .unwrap_or_default()
}

pub fn write_slot_state(root: &Path, s: &SlotState) -> std::io::Result<()> {
    let mut text = format!(
        "cart={}\ncart_platform={}\nbrightness={}\nblue_light={}\nvolume={}\nvolume_hp={}\nmuted={}\nmuted_hp={}\nclock_set={}\nutc_offset_min={}\nmode={}\nrumble={}\nff_speed={}\nff_sound={}\ncolour_correction={}\nshader_gba={}\nshader_gb={}\neject_save={}\nturbo={}\nrewind={}\ngb_palettes={}\ngb_palette={}\n",
        s.cart.as_deref().unwrap_or(""),
        s.cart_platform.map_or(String::new(), platform_key),
        s.brightness,
        s.blue_light,
        s.volume,
        s.volume_hp,
        s.muted as u8,
        s.muted_hp as u8,
        s.clock_set as u8,
        s.utc_offset_min,
        s.mode.word(),
        s.rumble as u8,
        s.ff_speed,
        s.ff_sound as u8,
        s.colour_correction as u8,
        s.shader_gba.name(),
        s.shader_gb.name(),
        s.eject_save as u8,
        s.turbo as u8,
        s.rewind as u8,
        s.gb_palettes as u8,
        s.gb_palette.core_name()
    );
    // One line per favourite rather than a delimited list, so a stem that happens to contain
    // the delimiter is not two carts in disguise: `split_once('=')` on the way back splits at
    // the first `=` only, so everything after it — `=`, space, comma and all — is the stem.
    for stem in &s.favorites {
        if !stem.is_empty() {
            text.push_str("favorites=");
            text.push_str(stem);
            text.push('\n');
        }
    }
    atomic_write(&state_path(root), text.as_bytes())
}

/// The keys both sides have always written — cart, brightness, blue light, volume, muted,
/// clock, offset — are still all or nothing. A file missing one of those is not one we wrote,
/// and inheriting the gap from the defaults would hide the corruption behind plausible values.
///
/// Everything added later is the other way round. `mode` and `favorites` arrived on cards that
/// already had a state file, and so did rumble, fast-forward, the shaders, the headphone
/// levels and the palette. A missing one of those reads as its default (headphone volume and
/// mute fall back to the speaker values, which is what a card from before headphones did).
/// A key this build has never heard of is skipped rather than treated as corruption, so a
/// file written by the other side of a merge still loads.
///
/// A value that *is* present and unreadable is still corruption when it is one of the original
/// keys, or a mode: `mode=sepia` means someone has been editing the file. The newer settings
/// fall back to their own default instead, so one bad `ff_speed` does not throw the cart away.
///
/// `favorites` repeats on purpose — one `favorites=` line per stem — so that arm adds to a set
/// rather than assigning.
fn parse(text: &str) -> Option<SlotState> {
    let mut cart = None;
    let mut cart_platform = None;
    let mut brightness = None;
    let mut blue_light = None;
    let mut volume = None;
    let mut volume_hp = None;
    let mut muted = None;
    let mut muted_hp = None;
    let mut clock_set = None;
    let mut utc_offset_min = None;
    let mut mode = None;
    let mut favorites = BTreeSet::new();
    let mut rumble = None;
    let mut ff_speed = None;
    let mut ff_sound = None;
    let mut colour_correction = None;
    let mut shader_gba = None;
    let mut shader_gb = None;
    let mut eject_save = None;
    let mut turbo = None;
    let mut rewind = None;
    let mut gb_palettes = None;
    let mut gb_palette = None;
    for line in text.lines().filter(|l| !l.is_empty()) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "cart" => cart = Some(value.to_string()),
            "cart_platform" => cart_platform = platform_value(value),
            "brightness" => brightness = Some(level(value, BRIGHTNESS_MAX)?),
            "blue_light" => blue_light = Some(level(value, BLUE_LIGHT_MAX)?),
            "volume" => volume = Some(level(value, VOLUME_MAX)?),
            "volume_hp" => volume_hp = Some(level(value, VOLUME_MAX)?),
            "muted" => muted = Some(level(value, 1)? == 1),
            "muted_hp" => muted_hp = Some(level(value, 1)? == 1),
            "clock_set" => clock_set = Some(level(value, 1)? == 1),
            "utc_offset_min" => utc_offset_min = Some(offset(value)?),
            // Written by this build and unreadable in it is still corruption: a misspelt word
            // here means someone has been editing the file, and the defaults are the safe
            // answer for the rest of it too.
            "mode" => mode = Some(Mode::from_word(value)?),
            "favorites" => {
                if !value.is_empty() {
                    favorites.insert(value.to_string());
                }
            }
            "rumble" => rumble = flag(value),
            "ff_speed" => ff_speed = ff_speed_value(value),
            "ff_sound" => ff_sound = flag(value),
            "colour_correction" => colour_correction = flag(value),
            "shader_gba" => shader_gba = Shader::parse(value).filter(|s| Shader::GBA.contains(s)),
            "eject_save" => eject_save = flag(value),
            "turbo" => turbo = flag(value),
            "rewind" => rewind = flag(value),
            "gb_palettes" => gb_palettes = flag(value),
            "gb_palette" => gb_palette = GbPalette::parse(value),
            "shader_gb" => shader_gb = Shader::parse(value).filter(|s| Shader::GB.contains(s)),
            _ => {}
        }
    }
    let cart = cart?;
    let fallback = SlotState::default();
    Some(SlotState {
        cart: (!cart.is_empty()).then_some(cart),
        cart_platform,
        brightness: brightness?,
        blue_light: blue_light?,
        volume: volume?,
        volume_hp: volume_hp.or(volume)?,
        muted: muted?,
        muted_hp: muted_hp.or(muted)?,
        clock_set: clock_set?,
        utc_offset_min: utc_offset_min?,
        mode: mode.unwrap_or_default(),
        favorites,
        rumble: rumble.unwrap_or(fallback.rumble),
        ff_speed: ff_speed.unwrap_or(fallback.ff_speed),
        ff_sound: ff_sound.unwrap_or(fallback.ff_sound),
        colour_correction: colour_correction.unwrap_or(fallback.colour_correction),
        shader_gba: shader_gba.unwrap_or(fallback.shader_gba),
        shader_gb: shader_gb.unwrap_or(fallback.shader_gb),
        eject_save: eject_save.unwrap_or(fallback.eject_save),
        turbo: turbo.unwrap_or(fallback.turbo),
        rewind: rewind.unwrap_or(fallback.rewind),
        gb_palettes: gb_palettes.unwrap_or(fallback.gb_palettes),
        gb_palette: gb_palette.unwrap_or(fallback.gb_palette),
    })
}

fn platform_key(platform: Platform) -> String {
    platform.dir_name().to_ascii_lowercase()
}

fn platform_value(value: &str) -> Option<Platform> {
    Platform::ALL
        .into_iter()
        .find(|p| value.eq_ignore_ascii_case(p.dir_name()))
}

fn offset(value: &str) -> Option<i16> {
    value
        .parse()
        .ok()
        .filter(|n| (UTC_OFFSET_MIN..=UTC_OFFSET_MAX).contains(n))
}

fn ff_speed_value(value: &str) -> Option<u8> {
    value.parse().ok().filter(|n| FF_SPEEDS.contains(n))
}

fn level(value: &str, max: u8) -> Option<u8> {
    value.parse().ok().filter(|n| *n <= max)
}

fn flag(value: &str) -> Option<bool> {
    level(value, 1).map(|n| n == 1)
}
