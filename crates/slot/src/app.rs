use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use slot_gfx::{System as ScreenSystem, OUT_H, OUT_W};
use slot_input::{Action, Btn, MUTE_CHORD_MS};
use slot_power::{Battery, Charge, LedState, LidPolicy, Power};
use slot_retro::LinkChannel;
use slot_store::{
    format_stamp, read_slot_state, scan_cached, write_slot_state, Cart, Core, GbPalette,
    Platform, Shader, SlotState, StateEntry, StateRing, System, Theme, BLUE_LIGHT_MAX,
    BRIGHTNESS_MAX, FF_SPEEDS, RING_MAX, VOLUME_MAX,
};
use slot_ui::lang;
use slot_ui::{
    board_from, board_zoom, draw_backdrop, draw_edge_glow, draw_empty_slot,
    draw_shortcut_row, draw_slot_name, draw_status, draw_sticker_at, draw_top_band,
    ease, grown, letters, lid_at, lid_from, lift_of, on_board, shelf_cart_at, ClockPicker, Draw,
    FfState, GbShell, Hud, HudKind, Icon, Letters, LinkBadge, Millis, Placed, Polaroids,
    PowerChoice, QuickMenu, QuickMenuFaces, QuickRow, QuickValue, Refusal, Shelf, SlotChrome,
    TexId, Toast, BOARD_W, BOARD_X, CART_W, CHIP_H, CHIP_U, CHIP_V, CHIP_W, HINT_EDGE, HINT_H,
    HOP_LIFT, MOUTH_H, PLATE_Y, SHADOW_H, SHADOW_W, SHELF_TITLE_H, SOCKET_H, SOCKET_U, SOCKET_V,
    SOCKET_W, STICKER_H, TURN_PAD,
};

use crate::audio::{Profile, Sfx};
use crate::core_picker::{Chip, CorePicker, Outcome, Press};
use crate::link_kind::{link_carried, link_kind, serial_option, LinkKind};
use crate::link_radio::{radio_jobs, LinkRole, RadioJob, RadioJobs};
use crate::link_screen::LinkSprites;
use crate::link_start::{link_port, LinkFail, LinkProgress, LinkStarter, LinkStep};
use crate::persist::{self, Snapshot};
use crate::video_mode::{self, VideoMode};

pub const INSERT_S: f32 = 0.73;
const INSERT_HOLD_S: f32 = 0.28;

pub const SEATED_AT: f32 = INSERT_S - INSERT_HOLD_S;

pub const EJECT_S: f32 = SEATED_AT;

const EJECT_HOLD_S: f32 = 0.35;

const POWER_ON_S: f32 = 0.22;

const POWER_OFF_S: f32 = 0.16;

const VOLUME_STEP: u8 = 5;

const AUTOSAVE_MS: Millis = 60_000;
const CARD_CHECK_MS: Millis = 2_000;

const BATTERY_CRITICAL: u8 = 5;

const BATTERY_POLL_MS: Millis = 10_000;

const CHARGE_POLL_MS: Millis = 1_000;
const HEADPHONES_POLL_MS: Millis = 500;

const BATTERY_LOW: u8 = 20;

pub const UNDO_GRACE_MS: Millis = 30_000;

pub const LINK_LOST_MS: Millis = 2000;

const PLAY_HOLD_MS: Millis = 500;

const SHUTDOWN_SHOW_MS: Millis = 250;

const POWER_MENU_PITCH: f32 = 44.0;
const POWER_MENU_BAR_INSET: f32 = 4.0;
/// The cheat table's row spacing and the most rows it shows before it scrolls. Smaller than the
/// power menu's pitch because codes are short and there are often many of them.
const CHEAT_PITCH: f32 = 44.0;
const CHEAT_VISIBLE: usize = 12;
/// How far the row makes way while a cart is open, as `Shelf::draw_row` counts `recede`. It is
/// set by where the neighbours stand: here they come to rest at -41 and 574, where the mockup
/// frames the open cart with them. Parted far enough for the recede alone to dim them to a
/// quarter, they left the open cart alone in the frame.
const CORE_PICKER_RECEDE: f32 = 0.26;

const SLOT_NAME_IN_MS: Millis = 200;
const SLOT_NAME_HOLD_MS: Millis = 1200;
const SLOT_NAME_OUT_MS: Millis = 800;
const SLOT_NAME_ALPHA: f32 = 0.4;
/// How much further the neighbours' faces darken while a cart is open, since the recede that
/// stands them in place dims them only part of the way. Just the recede leaves a side cart's
/// face at `SIDE_ALPHA * (1 - CORE_PICKER_RECEDE)`, and the mockup has the neighbours at a
/// quarter, so this is what is left to take off.
///
/// Derived from the shelf's own `SIDE_ALPHA` rather than written down as a number. It used to
/// be a literal, computed against the 0.55 `SIDE_ALPHA` of the day, and when the look was
/// re-cut to 0.7 the literal stayed put: the neighbours came to rest at a third instead of a
/// quarter, which the test for it caught.
const CORE_PICKER_DIM: f32 = 0.25 / (slot_ui::SIDE_ALPHA * (1.0 - CORE_PICKER_RECEDE));
/// The legend's line, under the open cart and clear of the case band.
const CORE_LEGEND_Y: f32 = 386.0;
const LINK_TEXT_Y: f32 = 44.0;
const LINK_LEGEND_Y: f32 = 422.0;
const LINK_LEGEND_GAP: f32 = 40.0;
const LID_SHADOW_W: f32 = 168.0;
const LID_SHADOW_H: f32 = 18.0;
const LID_SHADOW_DROP: f32 = 29.0;
const LID_SHADOW_ALPHA: f32 = 0.8;
const FACES_WAIT_MS: Millis = 1500;

const ALERT_HOLD: f32 = 0.45;
const ALERT_GONE: f32 = 0.9;

const CLOCK_FLOOR: i64 = 1_577_836_800;

pub enum PendingUndo {
    Save {
        stamp: String,
        evicted: Option<(String, Vec<u8>, Vec<u8>)>,
    },
    Load {
        prior: Vec<u8>,
    },
}

struct LinkSession {
    client_id: u16,
    lost_at: Option<Millis>,
}

struct LinkStarting {
    starter: LinkStarter,
    client_id: u16,
}

struct Reload {
    stem: String,
    role: LinkRow,
    cancelled: bool,
    from: LinkKind,
    from_serial: &'static str,
    fallback: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GameMenu {
    Pick(LinkRow),
    Working {
        role: LinkRow,
        step: LinkStep,
        since: Millis,
    },
    Linked {
        role: LinkRow,
        worked: Millis,
        since: Millis,
        opened: bool,
    },
    Failed {
        role: LinkRow,
        fail: LinkFail,
        worked: Millis,
        since: Millis,
    },
    Unplug {
        role: LinkRow,
        since: Millis,
    },
}


#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkRow {
    Host,
    Join,
}

impl LinkRow {
    pub const ALL: [LinkRow; 2] = [LinkRow::Host, LinkRow::Join];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn text(self) -> &'static str {
        match self {
            LinkRow::Host => lang::LINK_HOST,
            LinkRow::Join => lang::LINK_JOIN,
        }
    }

    pub fn role(self) -> LinkRole {
        match self {
            LinkRow::Host => LinkRole::Host,
            LinkRow::Join => LinkRole::Join,
        }
    }

    pub fn client_id(self) -> u16 {
        match self {
            LinkRow::Host => 0,
            LinkRow::Join => 1,
        }
    }

    pub fn other(self) -> LinkRow {
        match self {
            LinkRow::Host => LinkRow::Join,
            LinkRow::Join => LinkRow::Host,
        }
    }

    pub fn from_client_id(id: u16) -> LinkRow {
        if id == 0 {
            LinkRow::Host
        } else {
            LinkRow::Join
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LinkLegend {
    Cancel,
    Mode,
    Swap,
    Link,
    Ok,
    Back,
    EndLink,
}

impl LinkLegend {
    pub const ALL: [LinkLegend; 7] = [
        LinkLegend::Cancel,
        LinkLegend::Mode,
        LinkLegend::Swap,
        LinkLegend::Link,
        LinkLegend::Ok,
        LinkLegend::Back,
        LinkLegend::EndLink,
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

pub const LINKED_HOLD_MS: Millis = 1000;

pub const UNPLUG_HOLD_MS: Millis = 420;

#[derive(Debug)]
pub enum Phase {
    SetClock {
        picker: ClockPicker,
        seed: i64,
        from_menu: bool,
    },
    Shelf,
    QuickMenu {
        row: QuickRow,
    },
    Inserting {
        cart: String,
        t: f32,
        core_ready: bool,
        resumed: bool,
        clean: bool,
    },
    Playing {
        cart: String,
    },
    Ejecting {
        cart: String,
        t: f32,
    },
    Polaroids {
        cart: String,
    },
    /// The label, and the shortcut card hanging under it. A screen of its own rather than a
    /// panel, because it is one object being looked at and there is nothing else on it — but
    /// the object grew a list, so it carries how far down it has been scrolled and how far down
    /// the last press asked it to go. `from_menu` is set when the quick menu opened it, so
    /// closing returns there instead of to the shelf.
    About {
        scroll: f32,
        want: f32,
        from_menu: bool,
    },
    Doze {
        cart: Option<String>,
    },
}

/// Where the shelf's line of type sits: centred in the gap between the row's own foot and
/// the top of the case. Derived rather than picked, so moving either end — a taller cart, a
/// deeper bay — carries it along instead of leaving it behind.

/// How many carts fall under each slot of the letter ring, in `letters::SLOTS` order.
///
/// One pass at boot and never again: the card cannot change while slot is running. The strip
/// reads it to know which letters are worth stopping on and which are drawn dim, so it has to
/// be counted from the same list the row is drawn from rather than from anything on disk.
fn tally_letters(carts: &[Cart]) -> [usize; letters::N] {
    let mut counts = [0usize; letters::N];
    for cart in carts {
        counts[letters::slot_of(cart.initial)] += 1;
    }
    counts
}

/// Where the about card starts on the panel, and the gap between the label and the rows under
/// it. The label used to be centred because it was the whole screen; now that the card runs
/// past the bottom of the panel, it belongs at the top of what there is.
const ABOUT_PAD: f32 = 26.0;
const ABOUT_GAP: f32 = 20.0;

/// The first row's resting place: below the label and one gap past it. Derived from the two
/// numbers above rather than written down, so moving the label carries the rows with it.
fn about_first_y() -> f32 {
    ABOUT_PAD + STICKER_H as f32 + ABOUT_GAP
}

/// The bar the pinned foot line is set on: the panel's full width, opaque, flush with the
/// bottom edge. Drawn here rather than baked into the hint's own face, because it belongs to
/// the screen and not to the card — the card is 660 across, this is all 720.
const HINT_BAR_INK: [f32; 4] = [0.055, 0.05, 0.045, 1.0];
/// The bar's lit top edge: a few hairlines fading upward, which is as close as a list of flat
/// rects comes to a glow. Faint on purpose — it is a rim light, not a light.
const HINT_GLOW_STEPS: u32 = 8;
const HINT_GLOW_H: f32 = 1.5;
const HINT_GLOW_A: f32 = 0.13;
const HINT_GLOW_INK: [f32; 3] = [0.86, 0.83, 0.77];

/// How far one press takes the card. Six rows: less than that is a card the user has to walk to
/// the end of, and a whole screen is a card that jumps past the line it was being read at.
const ABOUT_PAGE: f32 = 180.0;

/// How fast the card closes on where the last press asked it to go, as a share of the way there
/// per second. Slower than the letter strip's spring on purpose: this is text being moved, and
/// there is nothing here that has to land on anything.
const ABOUT_EASE: f32 = 9.0;

/// The footer band and the faint light along its top edge. Opaque and the panel's full width,
/// so the rows scrolling under it pass behind it rather than through it. `top` is where the
/// band starts; the glow is laid above it, brightest against the edge.
fn draw_hint_bar(top: f32, out: &mut Vec<Draw>) {
    out.push(Draw::Rect {
        x: 0.0,
        y: top,
        w: OUT_W as f32,
        h: (OUT_H as f32 - top).max(0.0),
        colour: HINT_BAR_INK,
    });
    for i in 0..HINT_GLOW_STEPS {
        // Brightest next to the edge, dimming as it climbs away from it.
        let a = HINT_GLOW_A * (i as f32 + 1.0) / HINT_GLOW_STEPS as f32;
        let y = top - (HINT_GLOW_STEPS - i) as f32 * HINT_GLOW_H;
        out.push(Draw::Rect {
            x: 0.0,
            y,
            w: OUT_W as f32,
            h: HINT_GLOW_H,
            colour: [HINT_GLOW_INK[0], HINT_GLOW_INK[1], HINT_GLOW_INK[2], a],
        });
    }
}

/// Where 灰度 sits in `GRADES` — the one grade that keeps reading the card's own `cc.txt`, so it
/// needs naming rather than being found by counting.
pub const GRADE_GRAY: u16 = 9;

/// One entry of the colour-machine grade list — a name for the browser and boot log, and what
/// the grade actually is.
pub struct Grade {
    pub name: &'static str,
    pub kind: GradeKind,
}

/// How a grade makes its look. Four mechanisms, because that is what the good ones need.
pub enum GradeKind {
    /// Straight through.
    Off,
    /// A 3×3 and the gamma it runs under, both in linear space. The matrix already carries its
    /// profile's luminance: `lum` is a scalar applied to the linear colour *before* the matrix,
    /// so folding it into the entries is exact arithmetic rather than an approximation.
    Matrix([[f32; 3]; 3], f32),
    /// Two stops of a 1D ramp — dark then light — walked by the picture's own luma.
    Ramp([u8; 3], [u8; 3]),
    /// A baked 32³ lookup, laid out exactly like `u_pix`: 1024×32 RGBA, `idx = (b*1024 + g*32 + r)*4`.
    Lut(&'static [u8]),
}

/// The colour grades a GBA or GBC cycles, **page one then page two, nine to a page**. The order
/// is the one the player asked for, so the two pages are two moods: the first is "what the panel
/// was", the second is "what the picture can be made to look like".
pub const GRADES: [Grade; 18] = [
    // ---- page 1: 实机系 ----
    Grade { name: "无矫正", kind: GradeKind::Off },
    Grade { name: "AGB-001", kind: GradeKind::Matrix(DisplayFilter::AGB_001_CC, 2.2) },
    Grade { name: "libretro GBA", kind: GradeKind::Matrix(
        [[0.82355, 0.17745, -0.09100],
         [0.09100, 0.59150, 0.22750],
         [0.14333, 0.12968, 0.63700]], 2.2) },
    // VBA/No$GBA full colour: target 2.45 with a display gamma of 1.45, which one gamma cannot
    // hold exactly; 1.69 is the best single value and the look is a nostalgic one anyway.
    Grade { name: "VBA 全彩", kind: GradeKind::Matrix(
        [[0.73000, 0.27000, 0.00000],
         [0.08250, 0.67750, 0.24000],
         [0.08250, 0.24000, 0.67750]], 1.69) },
    Grade { name: "GBC 转置", kind: GradeKind::Matrix(
        [[0.88000, 0.05000, -0.03000],
         [0.07250, 0.60000, 0.35000],
         [0.02500, 0.05000, 1.20000]], 1.7) },
    Grade { name: "NDS", kind: GradeKind::Matrix(DisplayFilter::NDS_CC, 2.0) },
    Grade { name: "DB16", kind: GradeKind::Lut(include_bytes!("../assets/db16.bin")) },
    Grade { name: "DB32", kind: GradeKind::Lut(include_bytes!("../assets/db32.bin")) },
    Grade { name: "Commodore 64", kind: GradeKind::Lut(include_bytes!("../assets/c64.bin")) },
    // ---- page 2: 风格系 ----
    Grade { name: "灰度", kind: GradeKind::Matrix(DisplayFilter::DEFAULT_CC, 1.0) },
    Grade { name: "双色 深蓝米白", kind: GradeKind::Ramp([16, 24, 64], [246, 238, 214]) },
    Grade { name: "双色 紫青", kind: GradeKind::Ramp([48, 24, 80], [150, 245, 235]) },
    Grade { name: "双色 墨绿淡黄", kind: GradeKind::Ramp([18, 44, 32], [238, 232, 168]) },
    Grade { name: "双色 棕奶白", kind: GradeKind::Ramp([58, 34, 22], [250, 226, 190]) },
    Grade { name: "TealOrange", kind: GradeKind::Lut(include_bytes!("../assets/tealorange.bin")) },
    Grade { name: "HorrorBlue", kind: GradeKind::Lut(include_bytes!("../assets/horrorblue.bin")) },
    Grade { name: "Classic Chrome", kind: GradeKind::Lut(include_bytes!("../assets/classicchrome.bin")) },
    Grade { name: "BleachBypass", kind: GradeKind::Lut(include_bytes!("../assets/bleachbypass2.bin")) },
];

/// What a colour grade does to **one colour** — the same arithmetic the game pass does, run on a
/// single pixel so the browser can show what a grade is before it is picked.
///
/// A Game Boy's entries carry their own four shades, so a cell shows the palette itself; a grade
/// has no such list, so it is run instead. **It used to be run on the grey axis** — `[0, 85, 170,
/// 255]` and every step between — which answers "what does this grade do to white and black", and
/// most grades leave those alone: every entry came out looking alike and grey, and the player said
/// exactly that. A hue is what a picture is made of, so a hue is what is fed in (`SWEEP`), and
/// three of the four mechanisms below can only be told apart on one:
///
/// * a matrix moves colours, so a sweep shows it as a wash or a cast;
/// * a palette grade collapses hues onto its own colours, so the sweep becomes a stack of them;
/// * a baked cube is a *tone* map as well, so the tone strip under the sweep is where its split
///   shows — `TealOrange` leaves a saturated dark red red and puts its teal in the neutral
///   shadows, so a second row of hues would have shown nothing of it.
pub fn grade_rgb(g: &Grade, rgb: [u8; 3]) -> [u8; 3] {
    let mut out = [0u8; 3];
    match g.kind {
        GradeKind::Off => out = rgb,
        // In linear space, and as the real 3x3 rather than a row sum: with a grey input every
        // column is the same number and the sum was correct, but a hue has three different ones.
        GradeKind::Matrix(m, gamma) => {
            let lin = rgb.map(|c| (c as f32 / 255.0).powf(gamma));
            for row in 0..3 {
                let x = m[row][0] * lin[0] + m[row][1] * lin[1] + m[row][2] * lin[2];
                out[row] = (x.clamp(0.0, 1.0).powf(1.0 / gamma) * 255.0).round() as u8;
            }
        }
        // Walked by the picture's own luma, Rec.601 weights, exactly as the shader samples
        // `ramp_lut`'s 256 entries.
        GradeKind::Ramp(dark, light) => {
            let l = 0.299 * rgb[0] as f32 + 0.587 * rgb[1] as f32 + 0.114 * rgb[2] as f32;
            for c in 0..3 {
                let (a, b) = (dark[c] as f32, light[c] as f32);
                out[c] = (a + (b - a) * (l / 255.0)).clamp(0.0, 255.0).round() as u8;
            }
        }
        // The cube is 32 per channel, `idx = (b*1024 + g*32 + r)*4`, so a colour is quantised to
        // the cell of the cube it lands in — which is what the shader's own lookup does.
        GradeKind::Lut(bytes) => {
            let q = rgb.map(|c| (c as usize * 31 + 127) / 255);
            let at = (q[2] * 1024 + q[1] * 32 + q[0]) * 4;
            out = [bytes[at], bytes[at + 1], bytes[at + 2]];
        }
    }
    out
}

/// A colour grade's browser cell: **the grade, run over a picture's colours** — `SWEEP`'s eight
/// hues with `TONE`'s black-to-white strip under them, every stop through `grade_rgb`.
///
/// The cell is the grade rather than a drawing of it, which is what makes the browser worth
/// opening: a grade that washes colour out washes the cell out, a palette grade snaps it onto that
/// palette's own colours, and a split-tone pulls the two ends of the tone strip apart — teal at
/// one end and orange at the other, which is why it is called that.
pub fn grade_swatch(g: &Grade) -> slot_ui::PaletteSwatch {
    slot_ui::PaletteSwatch::chart(|rgb| grade_rgb(g, rgb))
}

/// The 1D ramp a duotone walks: 256 RGBA entries, dark at the picture's black and light at its
/// white, interpolated in the encoded space the shader samples with (see `GAME_FRAG`).
fn ramp_lut(dark: [u8; 3], light: [u8; 3]) -> [u8; 1024] {
    let mut lut = [0u8; 1024];
    for i in 0..256usize {
        for c in 0..3 {
            let (a, b) = (dark[c] as u32, light[c] as u32);
            lut[i * 4 + c] = ((a * (255 - i as u32) + b * i as u32 + 127) / 255) as u8;
        }
        lut[i * 4 + 3] = 255;
    }
    lut
}

/// The on-screen picture's two independent knobs.
///
/// Two ways to the same end — a screen the picture is seen *through* — and the machine picks:
///
/// **A mesh**, drawn in the game shader (`display_grid`), on the two machines with a screen
/// bezel: a Game Boy's is its palette's lightest shade and always on, a Game Boy Color's is
/// white or black and rides the overlay ring.
///
/// **An aperture table** (`lcd3x`, `applied_mask`), on the Advance: a 3x3 of per-subpixel
/// multipliers. It is the one road to a real LCD subpixel structure, which a mesh cannot be —
/// and it is a *multiply*, so it can only darken, which is why the bezel machines draw a mesh
/// instead (a white grid needs a mix). `SELECT+X` steps its `MASK_STATES` presets — off, the
/// LCD3x table at four strengths, and the scanline at two — and the position is the first
/// number of `display.txt`.
///
/// `cc` picks the colour correction, and *that* list is shared: the two colour machines (GBA and
/// GBC) walk one table of **`GRADES`**, and a Game Boy walks its own palette table instead (see
/// `gb_lut`), because a greyscale machine's "colour" is a palette rather than a grade. The shared
/// list is screen grades rather than saturations — first what the panel was, then what the
/// picture can be made to look like. Each is cycled on its own chord — SELECT+X for that
/// machine's own screen look (the overlay ring where there is art, the grid ring where there is
/// none), SELECT+Y for the colour — and the lot is persisted to `System/display.txt` as
/// "grid cc_gba cc_gb cc_gbc", the first being the Advance's ring.
///
/// AGB-001 与 NDS 在**线性空间**里做（`CC_GAMMA`）：直接在编码空间乘会把画面压暗、冲淡；
/// 转线性、乘完再转回，观感才对（RetroArch 手持着色器同法）。灰度档走 gamma 1.0，行为与旧版一致。
struct DisplayFilter {
    /// The card's LCD3x table if it ships one, else the built-in. **Kept but unbound** with the
    /// aperture it feeds — see the struct doc.
    #[allow(dead_code)]
    lcd3x: [[[u8; 3]; 3]; 3],
    /// The card's colour matrix (from `cc.txt`) if it ships one, else the NOCOLOR (0% saturation)
    /// default. Only used when `cc_mode` is 2. The matrix the game pass multiplies the picture by,
    /// in the spirit of a colour-saturation shader but applied as a single 3x3 multiply so it
    /// costs nothing here.
    nocolor_cc: [[f32; 3]; 3],
    /// The Advance's aperture choice, 0..=4. Read by nothing else — see the struct doc.
    /// The Advance's aperture preset, `0..MASK_STATES` — what SELECT+X steps.
    mask_mode: u8,
    /// One colour-correction choice per machine, in `MACHINE_SLOTS` order (GBA, GB, GBC).
    ///
    /// Per machine because the three machines want different *kinds* of correction and there is
    /// no sensible shared index into them. A Game Boy is a greyscale machine: saturation is
    /// meaningless on it, and what its four shades are tinted with is the whole question — DMG
    /// green, Pocket grey, Light cyan. A Game Boy Color and an Advance are colour machines,
    /// where saturation and a backlight tint both mean something. One number shared between them
    /// would have to mean "entry 3 of whichever list", which is exactly the kind of thing that
    /// reads fine until someone cycles it on the wrong shelf.
    ///
    /// `u16` rather than `u8` because the Game Boy's slot is an index into the whole palette
    /// table, which is past 255 entries and will only grow. The two colour machines' slots still
    /// hold 0..=6; the width is shared because the three are read as one array.
    cc: [u16; 3],
}

/// How many machines have their own shelf and their own colour correction. The three the
/// device ships with; `machine_slot` is the mapping and it is the only one.
const MACHINE_SLOTS: usize = 3;
impl DisplayFilter {
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    // GRAYSCALE (0% saturation): plain luma. The whole picture collapses to its luma
    // (Rec.601 weights 0.299/0.587/0.114), so colours keep their brightness but lose hue
    // entirely — the "black & white / 灰度" look. This is mode 0. Overridable per card through
    // System/cc.txt; this is the fallback. Uploaded row-major -> column-major by
    // `Fbo::set_color_correction`, so this multiplies as written.
    const DEFAULT_CC: [[f32; 3]; 3] = [
        [0.299, 0.587, 0.114],
        [0.299, 0.587, 0.114],
        [0.299, 0.587, 0.114],
    ];
    /// AGB-001 (mode 1): the authentic original Game Boy Advance screen. The AGB-001 / AGS-001
    /// panel had a small, non-sRGB gamut, so games — mastered oversaturated to fight the dim
    /// reflective screen — look washed out, faintly green, and dark on it. This is a desaturation
    /// to s = 0.6 (luma blend, Rec.601 weights) with a mild green cast (blue pulled down, green
    /// nudged up), run in linear space under `CC_GAMMA` (2.2) so the midtones darken as the real
    /// panel did. Values are a first pass from the colour science (see the Handheld Color Space
    /// Project); every entry here is the whole look, so tune to taste.
    const AGB_001_CC: [[f32; 3]; 3] = [
        [0.7052, 0.2301, 0.0447],
        [0.1220, 0.8515, 0.0465],
        [0.1100, 0.2160, 0.5939],
    ];
    /// NDS (mode 2): the Nintendo DS Phat screen — a softer, near-sRGB middle ground between the
    /// washed-out GBA and modern displays. Desaturation to s = 0.88 (mild), no tint, run under a
    /// slightly lower gamma (2.0) than AGB-001 so it reads brighter, matching the DS Phat's
    /// backlit panel. Pokefan531's `nds-color` shader is the reference.
    const NDS_CC: [[f32; 3]; 3] = [
        [0.91588, 0.07044, 0.01368],
        [0.03588, 0.95044, 0.01368],
        [0.03588, 0.07044, 0.89368],
    ];

    /// 线性空间里做色彩校正所用的 gamma（对应 `GAME_FRAG` 的 `u_cc_gamma`）。
    /// 2.2 = sRGB。AGB-001/NDS 两档在线性空间里做（游戏 ROM 是编码空间、直接乘会被压暗/冲淡）；
    /// 灰度(0) 传 1.0，仍是编码空间直乘，与旧版逐位一致。
    pub const CC_GAMMA: f32 = 2.2;
    /// Which shelf a machine's colour choice lives on.
    const fn machine_slot(s: System) -> usize {
        match s {
            System::Gba => 0,
            System::Gb => 1,
            System::Gbc => 2,
        }
    }

    /// How many colour-correction entries the colour machines (GBA, GBC) have: grayscale, AGB-001,
    /// NDS, pixel-art — four. The Game Boy has its own count (the whole palette table).
    pub const CC_COUNT: u16 = GRADES.len() as u16;
    /// The pixel-art grade's index in that list — the one that quantises to `PIXEL_PALETTE`.
    pub const PIXEL: u16 = 3;
    /// How many the Game Boy has: the whole palette table, which is every PixelShift and TWB64
    /// entry the core ships plus the two house looks in front of them. See `crate::palettes`.
    pub const GB_CC_COUNT: u16 = crate::palettes::GB_PALETTES.len() as u16;

    /// The Game Boy's palettes — the whole table, as `(name, four shades)` — live in
    /// `crate::palettes`, generated from gambatte's own `gbcpalettes.h`.
    ///
    /// The table is four shades rather than a matrix, because four shades is what the machine
    /// has. A 3x3 multiply can only slope between one white point and one black point: it can
    /// tint, and it cannot follow a palette whose middles sit off that line. `Sunburst` runs pale
    /// green -> yellow -> orange -> red, and a two-point map draws a straight line from one end to
    /// the other and loses both of the middle colours. `gb_lut` turns a table like this into a
    /// per-shade map instead, and the shader looks it up.
    ///
    /// Entry 0 原生灰 is the core's own output, untouched — and it is deliberately *not* routed
    /// through the lookup, so the machine's neutral look costs nothing.

    /// The three Game Boy models — the screen art **and** the four shades behind it — live in
    /// `crate::palettes::GB_MODELS`.
    ///
    /// They are not entries in the palette table, and that is the whole of what "bound" means:
    /// each model's four shades were drawn as the other half of its screen art, so the choice is
    /// made once, by choosing the art. Letting them in as palettes as well would be two ways to
    /// land on one look.
    ///
    /// How many of a Game Boy's overlay choices are models: they come first, so a card that ships
    /// the three masks opens on the DMG.
    pub const GB_MODEL_COUNT: usize = 3;

    /// How much of the lattice colour is mixed into the dots: half.
    ///
    /// The geometry is the shader's — the right column and bottom row of every 3x3 source pixel,
    /// 5 of its 9 — and at that coverage the mesh is much heavier than the old aperture table
    /// ever was, so this is the one number that decides whether the picture survives it. Tried
    /// at 35% and set back to 50% on the device; it is a taste dial and nothing else depends
    /// on the value.
    pub const GB_GRID_MIX: f32 = 0.5;
    /// How strongly the Game Boy Color's lattice shows, in either colour. Three tenths: a half read
    /// as a veil over the picture rather than as the glass in front of it, and a quarter was a
    /// shade too faint.
    pub const GBC_GRID_MIX: f32 = 0.30;
    /// How many states each Game Boy Color overlay carries on the ring: **without the grid, with a
    /// white one, and with a black one**. The count is what ties `overlay_choices`,
    /// `ordinary_overlay_at` and `display_grid` together, so it lives in one place.
    pub const GBC_GRID_STATES: usize = 3;
    /// How many apertures the Advance's ring holds: OFF, four strengths of the LCD3x table, and the
    /// scanline at two. **The number itself lives in `root`,** because `display.txt`'s first field
    /// indexes it and that is the file that has to clamp against it; this is the ring agreeing with
    /// the card rather than the other way round.
    pub const MASK_STATES: usize = crate::root::MASK_STATES;

    /// The palette's own name is not a second array beside the table: each entry of
    /// `crate::palettes::GB_PALETTES` is `(name, four shades)`, so a name cannot drift from the
    /// colours it names. `display_palette_name` is the accessor.

    /// Where the four shades sit in the greys the core hands over. Not evenly spaced: mGBA's
    /// greyscale is its own four levels (RGB555 `0x7FFF / 0x56B5 / 0x294A / 0x0000`, which is
    /// 255 / 173 / 82 / 0 once widened), and the lookup has to put each shade's colour *on* its
    /// level rather than on an idealised third of the range.
    const GB_SHADE_LEVELS: [u8; 4] = [255, 173, 82, 0];

    /// Turn four shades into a 256-entry RGBA lookup, so the shader can be exact.
    ///
    /// Every grey the machine can hand over gets a colour, interpolated between the two shades
    /// either side of it. Interpolating matters even though a Game Boy only draws four levels:
    /// the reflection pass feeds this a *blurred* frame, which is continuous, and a lookup that
    /// only knew about four values would posterise the glow around the screen.
    ///
    /// Built in encoded space, the space the panels are described in, so a shade lands on its
    /// own colour exactly rather than a gamma away from it.
    ///
    /// Takes the shades themselves rather than an index into the table: the table's entries and
    /// the three models' four shades are the same kind of thing here, and only one of the two has
    /// an index at all.
    fn gb_lut(pal: [[u8; 3]; 4]) -> [u8; 1024] {
        let lv = Self::GB_SHADE_LEVELS;
        let mut lut = [0u8; 1024];
        for g in 0..256u32 {
            // Find the pair of shades this grey falls between. `lv` runs bright to dark and `g`
            // runs dark to bright, so the search walks the other way.
            let mut lo = 0;
            while lo < 3 && (g as u8) < lv[lo + 1] {
                lo += 1;
            }
            // Between lv[lo] (brighter) and lv[lo + 1] (darker).
            let (hi_lv, lo_lv) = (lv[lo] as u32, lv[(lo + 1).min(3)] as u32);
            let t = if hi_lv == lo_lv {
                0
            } else {
                ((hi_lv - g) * 255 / (hi_lv - lo_lv)).min(255)
            };
            let a = pal[lo];
            let b = pal[(lo + 1).min(3)];
            let i = (g * 4) as usize;
            for c in 0..3 {
                let (x, y) = (a[c] as u32, b[c] as u32);
                lut[i + c] = ((x * (255 - t) + y * t + 127) / 255) as u8;
            }
            lut[i + 3] = 255;
        }
        lut
    }

    /// The neutral aperture the LCD3x modes sit between: an everywhere-white table, so OFF is a
    /// clean framebuffer and the two LCD3x steps are the grid at half and full strength.
    const FLAT_MASK: [[[u8; 3]; 3]; 3] = [[[255u8; 3]; 3]; 3];
    /// A horizontal scanline overlay: the top row of each 3-line cell is the dark gap, the other
    /// two are lit. Used at full strength for mode 4; mode 3 is this lerped halfway from FLAT.
    const SCANLINE_MASK: [[[u8; 3]; 3]; 3] = [[[0u8; 3]; 3], [[255u8; 3]; 3], [[255u8; 3]; 3]];
    fn new(
        lcd3x: [[[u8; 3]; 3]; 3],
        nocolor_cc: [[f32; 3]; 3],
        mode: u8,
        cc: [u16; 3],
    ) -> Self {
        Self {
            lcd3x,
            nocolor_cc,
            // One stored index, one ring: the aperture preset whose position `SELECT+X` steps.
            mask_mode: mode.min(Self::MASK_STATES as u8 - 1),
            cc: [
                cc[0].min(Self::CC_COUNT - 1),
                cc[1].min(Self::GB_CC_COUNT - 1),
                cc[2].min(Self::CC_COUNT - 1),
            ],
        }
    }
    /// The aperture table for the machine in hand. The two machines with a screen bezel answer
    /// `flat` — they draw their mesh in the shader instead — and the Advance reads the table its
    /// preset picks.
    fn applied_mask(&self, flat: bool) -> [[[u8; 3]; 3]; 3] {
        // A Game Boy's lattice is not a table any more: it is drawn from the palette in the game
        // shader (see `GB_GRID_MIX`), which is what makes it follow the four shades. The aperture
        // here is therefore neutral, and the mesh and the picture are made of the same colours.
        if flat {
            return Self::FLAT_MASK;
        }
        match self.mask_mode {
            0 => Self::FLAT_MASK,
            1 => lerp_mask(&Self::FLAT_MASK, &self.lcd3x, 0.25),
            2 => lerp_mask(&Self::FLAT_MASK, &self.lcd3x, 0.50),
            3 => lerp_mask(&Self::FLAT_MASK, &self.lcd3x, 0.75),
            4 => self.lcd3x,
            5 => lerp_mask(&Self::FLAT_MASK, &Self::SCANLINE_MASK, 0.50),
            _ => Self::SCANLINE_MASK,
        }
    }
    /// Colour correction rides under the mask, and what it *is* depends on the machine.
    ///
    /// A Game Boy gets a palette: see `gb_cc`, whose black point is the reason this returns a
    /// bias at all. The colour machines keep a short list of screen grades — 0 the grayscale luma,
    /// 1 the AGB-001 look, 2 the NDS look — all anchored on black, so their bias is the zero
    /// vector and their picture is bit for bit what it was before this pair existed.
    fn applied_cc(&self, machine: System) -> ([[f32; 3]; 3], [f32; 3]) {
        let mode = self.cc[Self::machine_slot(machine)];
        if machine == System::Gb {
            // A Game Boy is drawn by its palette lookup, not by a matrix: see `gb_lut`. The
            // matrix path stays identity so nothing double-applies when the lookup is on.
            return (Self::IDENTITY, [0.0, 0.0, 0.0]);
        }
        let grade = &GRADES[(mode as usize).min(GRADES.len() - 1)];
        let m = match grade.kind {
            // 灰度 keeps the card's own `cc.txt` matrix when it ships one, so that one grade stays
            // overridable per card; every other entry reads its own numbers.
            GradeKind::Matrix(_, _) if mode == GRADE_GRAY => self.nocolor_cc,
            GradeKind::Matrix(m, _) => m,
            _ => Self::IDENTITY,
        };
        (m, [0.0, 0.0, 0.0])
    }
    /// None for every machine now: the colour grades (grayscale / AGB-001 / NDS) have no single
    /// signature colour to fill the letterbox with, and a Game Boy palette leaves it black too.
    /// The surround stays black, which is what the real panels looked like.
    fn cc_border(&self, _machine: System) -> Option<[f32; 3]> {
        None
    }
    /// Step the aperture preset, wrapping at the end of the ring.
    fn cycle_mask(&mut self) {
        self.mask_mode = (self.mask_mode + 1) % Self::MASK_STATES as u8;
    }
    /// Step the colour correction of one of the **colour** machines, wrapping at the end of its
    /// list of three (grayscale, AGB-001, NDS).
    ///
    /// A Game Boy never comes through here: its palettes are a table hundreds of entries long
    /// walked a page at a time, and which page that is belongs to `App`, not to the table of
    /// three this struct also holds. `App::cycle_cc` is the fork.
    fn cycle_cc(&mut self, machine: System) {
        let slot = Self::machine_slot(machine);
        self.cc[slot] = (self.cc[slot] + 1) % Self::CC_COUNT;
    }
}

/// DawnBringer's 16-colour palette (DB16) — the classic limited palette of pixel art, and the
/// one the pixel-art grade snaps the picture onto. Sixteen colours is deliberately brutal: the
/// point is the posterised, hand-picked look, not a faithful screen.
const PIXEL_PALETTE: [[u8; 3]; 16] = [
    [0x14, 0x0c, 0x1c], [0x44, 0x24, 0x34], [0x30, 0x34, 0x6d], [0x4e, 0x4a, 0x4e],
    [0x85, 0x4c, 0x30], [0x34, 0x65, 0x24], [0xd0, 0x46, 0x48], [0x75, 0x71, 0x61],
    [0x59, 0x7d, 0xce], [0xd2, 0x7d, 0x2c], [0x85, 0x95, 0xa1], [0x6d, 0xaa, 0x2c],
    [0xd2, 0xaa, 0x99], [0x6d, 0xc2, 0xca], [0xda, 0xd4, 0x5e], [0xde, 0xee, 0xd6],
];

/// The pixel-art lookup, built once and kept for the life of the process.
static PIXEL_LUT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

/// The baked 32^3 lookup the pixel grade reads.
///
/// 131072 bytes (1024x32 RGBA): for every RGB555 level a GBA can emit, the nearest
/// `PIXEL_PALETTE` colour, laid out so the shader can fetch it with one NEAREST tap
/// (x = r + 32*g, y = b). Matched in encoded space — the space the core's output and the
/// palette are both described in — so what the eye compares is what the machine compares.
fn pixel_lut() -> &'static [u8] {
    PIXEL_LUT.get_or_init(|| {
        let pal = PIXEL_PALETTE;
        let palf: Vec<[f32; 3]> = pal
            .iter()
            .map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0])
            .collect();
        let mut lut = vec![0u8; 1024 * 32 * 4];
        for b in 0..32u32 {
            for g in 0..32u32 {
                for r in 0..32u32 {
                    let q = [r as f32 / 31.0, g as f32 / 31.0, b as f32 / 31.0];
                    let mut best = 0usize;
                    let mut bd = f32::MAX;
                    for (i, c) in palf.iter().enumerate() {
                        let d = (q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2) + (q[2] - c[2]).powi(2);
                        if d < bd {
                            bd = d;
                            best = i;
                        }
                    }
                    let c = pal[best];
                    let idx = ((b * 1024 + (g * 32 + r)) * 4) as usize;
                    lut[idx] = c[0];
                    lut[idx + 1] = c[1];
                    lut[idx + 2] = c[2];
                    lut[idx + 3] = 255;
                }
            }
        }
        lut
    })
}

/// Linearly blend two 3x3 aperture tables, element by element, at `t` in [0,1]. Used to soften
/// the LCD3x grid to a 50% strength for the middle mask step.
fn lerp_mask(a: &[[[u8; 3]; 3]; 3], b: &[[[u8; 3]; 3]; 3], t: f32) -> [[[u8; 3]; 3]; 3] {
    let mut out = [[[0u8; 3]; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                let av = a[i][j][k] as f32;
                let bv = b[i][j][k] as f32;
                out[i][j][k] = (av + (bv - av) * t).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// One cheat code for the seated cart, plus whether it is live. `enabled` is per-code; the
/// master `cheats_on` in `App` gates the lot — a code switched on but under a master that is
/// off does nothing, exactly as a global toggle over a picker of switches should.
pub(crate) struct CheatItem {
    pub(crate) code: String,
    pub(crate) desc: String,
    pub(crate) enabled: bool,
}

/// Inset of the shelf's star indicator from the corner of the case. Clear of the cart bay,
/// which owns the true bottom edge of the screen.

/// The machine logo in the bottom-left corner — the mirror of the favourites star in the
/// bottom-right — and the size it is drawn at. Only ever one logo: it *is* the shelf, and it
/// swaps with it.
const SHELF_IND_PX: f32 = 22.0;
const SHELF_IND_MARGIN: f32 = 12.0;
/// The bottom bar's type badge: what a card's own art is fitted into, and the size the built-in
/// word is drawn at. The card's art is 150 x 50, so half of it lands in this box exactly.
const SHELF_IND_W: f32 = 75.0;
const SHELF_IND_H: f32 = 25.0;
/// How far in from the panel's left edge the badge sits. Flush was too loud for a mark this
/// small: it read as a cut-off rather than as a badge.
const SHELF_IND_LEFT: f32 = 20.0;
/// How tall the bottom-right favourites indicator is drawn. Enlarged on request, to match the
/// mark a favourite cart wears.
const FAV_IND_DRAW: f32 = 36.0;
/// The top band's own margin: the distance the clock and the battery reading are printed in from
/// their edges. Repeated here so the two bottom corner marks can stand in the same columns as the
/// two readings above them.
const STATUS_READ_MARGIN: f32 = 24.0;

/// The shell that brings WiFi up and starts the SSH server, on demand rather than at boot: the
/// radio is a real drain on a standby budget, so it is the user who asks for it. `ags-net ssh`
/// loads `8821cs`, raises `wlan0`, joins from `System/wifi.conf` and starts `dropbear`; the `udhcpc`
/// after it is the address the firmware's own helper sometimes misses. The address is left in
/// `/tmp/slot-ip` for the dialog to show, since DHCP is what decides it.
const DIRECT_ENTER: &str = "sync; export PATH=/usr/sbin:/usr/bin:/sbin:/bin:$PATH; \
/usr/sbin/ags-net ssh >/dev/null 2>&1; udhcpc -i wlan0 -n -q -t 10 >/dev/null 2>&1; \
ip -4 -o addr show wlan0 2>/dev/null | tr -s ' ' | cut -d' ' -f4 | cut -d/ -f1 > /tmp/slot-ip; sync";

/// The shell that takes the radio back down: the SSH server, the supplicant and the DHCP client
/// are killed, `wlan0` dropped and the driver unloaded, which is what makes the drain go away
/// again. The card is never touched — the whole point of doing this over the network.
const DIRECT_EXIT: &str = "sync; export PATH=/usr/sbin:/usr/bin:/sbin:/bin:$PATH; \
pkill dropbear 2>/dev/null; pkill udhcpc 2>/dev/null; \
wpa_cli -i wlan0 terminate 2>/dev/null; pkill wpa_supplicant 2>/dev/null; \
ip link set wlan0 down 2>/dev/null; rmmod 8821cs 2>/dev/null; rm -f /tmp/slot-ip; sync";

/// The two beats of a system-shelf swap. The old shelf is drawn up and away over the first,
/// the new one drops back into place over the second, and the app takes no input for their sum.
/// Short on purpose: this is a shelf change, not a cutscene.
const SWITCH_ASCEND_S: f32 = 0.34;
const SWITCH_DESCEND_S: f32 = 0.34;

/// A system-shelf swap in flight: which shelf is arriving, how far through the two beats it is,
/// and whether the view has been swapped yet. While `App::shelf_switch` holds one, the app
/// takes no input and the row is drawn with the swap motion — see `slot_ui::Motion`.
struct ShelfSwitch {
    /// The system whose shelf is leaving. Held for the length of the swap so the corner logo can
    /// cross-dissolve from it rather than cut.
    from: System,
    /// The system whose shelf comes up when the ascend beat ends.
    to: System,
    /// Seconds since the swap began, across both beats.
    t: f32,
    /// Whether the view has been swapped to `to`. The row is the old shelf before it, the new
    /// one after.
    swapped: bool,
}

pub struct App {
    phase: Phase,
    shelf: Shelf,
    /// The letter strip over the shelf: where the ring stands, and how far it still has to
    /// travel.
    letters: Letters,
    /// How many carts sit under each of the ring's slots, in `letters::SLOTS` order. Built
    /// once from the scan and never again — the card does not change while slot is running.
    /// The strip needs it to know which letters are worth stopping on, and which are drawn dim.
    letter_counts: [usize; letters::N],
    /// One small face per slot, in that order with the size each was rasterised at. Uploaded at
    /// boot: 27 letters of a fixed alphabet is less work than one cart face.
    ///
    /// The band the letters stand in is not here at all: it is the machine's own, and
    /// `slot_chrome::draw_top_band` draws it from the cart bay's numbers every frame.
    letter_faces: Vec<(TexId, u32, u32)>,
    /// The one piece of metal the strip repeats between two letters, with its own size.
    /// Uploaded beside the letters because it is the same kind of thing: fixed, tiny, and
    /// needed on the shelf from the first frame.
    letter_ridge_face: Option<(TexId, u32, u32)>,
    /// When A went down on the shelf, and `None` the rest of the time. The hold lives here
    /// rather than in the gesture layer because A is the GBA's A button everywhere else, and
    /// `Gestures` is deliberately blind to which screen is up.
    play_held: Option<Millis>,
    /// When X went down on the shelf, and `None` the rest of the time — the same shape as
    /// `play_held`, and for the same reason. X carries two shelf gestures, a short press that
    /// stars the cart and a hold that swaps the shelf, and which one a press is cannot be known
    /// until it either comes up or passes `PLAY_HOLD_MS`.
    fav_held: Option<Millis>,
    /// When Y went down on the shelf: a hold cycles which machine's shelf is up. Same shape as
    /// `fav_held`, and spent the same way.
    sys_held: Option<Millis>,
    /// When B went down on the shelf: a hold opens the direct-connect dialog. A tap does nothing.
    mtp_held: Option<Millis>,
    /// The mark a favourite cart wears, with the size it was rasterised at. One colour whatever
    /// shelf is up. Its own upload rather than one of the HUD's, because it is the one glyph on the
    /// shelf that is a colour of its own instead of the case's ink.
    fav_star: Option<(TexId, u32, u32)>,
    /// The shelf's indicator, unlit and lit: the same star hollow while the ordinary shelf is
    /// up and solid while the favourites are.
    fav_ind: Option<((TexId, u32, u32), (TexId, u32, u32))>,
    /// The last refused action, and the only thing that tells an eject apart from a cart
    /// that would not seat: both leave down the same path.
    refusal: Option<Refusal>,
    refused_from: Option<f32>,
    alert_face: Option<TexId>,
    /// The game under the eye on the shelf, rasterised by the frontend whenever the selection
    /// moves — one line under the row naming what the row is showing. The cart carries a
    /// printed label of its own, but that label is a 240 px cart's: at the distance a handheld
    /// is held it says which of three carts is which, not which game this is, and a long title
    /// on it is three small lines rather than a name.
    shelf_title_face: Option<(TexId, u32, u32)>,
    /// What the shutdown says, one per `PowerChoice::ALL` in that order and rastered at the
    /// menu's own size. "Powering down" under a restart was the screen contradicting the row
    /// the user had just chosen.
    shutdown_faces: Vec<(TexId, u32, u32)>,
    power_menu: Option<usize>,
    power_menu_faces: Vec<(TexId, u32, u32)>,
    /// The direct-connect (USB mass-storage) dialog: which stage it is showing, or `None` when it
    /// is not up. `0` is the confirm prompt, `1` the "connect the cable" beat, `2` the live mode.
    direct_link: Option<u8>,
    /// The dialog's lines, baked once at boot: [confirm title, confirm keys, connecting, active
    /// title, active hint].
    direct_faces: Vec<(TexId, u32, u32)>,
    /// When the "connect the cable" beat went up, so it can hand over to the live screen on its own.
    direct_at: Millis,
    /// The address the WiFi link came up on, once the connect script has left one.
    direct_ip: Option<String>,
    /// The baked face for that address, and whether the address has changed since it was baked.
    direct_ip_face: Option<(TexId, u32, u32)>,
    direct_ip_dirty: bool,
    /// The picker while the cart is open, and while its lid is going back on. The cart it acts
    /// on is whichever the shelf has, read when it opens rather than held here: the shelf cannot
    /// move while it is up, so there is only ever one answer.
    core_picker: Option<CorePicker>,
    core_board_face: Option<TexId>,
    core_lid_face: Option<TexId>,
    core_faces_stem: Option<String>,
    core_socket_faces: Vec<TexId>,
    core_chip_faces: Vec<TexId>,
    core_blank_chip_face: Option<TexId>,
    core_chip_shadow_face: Option<TexId>,
    core_legend_faces: Vec<(TexId, u32)>,
    game_menu: Option<GameMenu>,
    link_sprites: Option<LinkSprites>,
    link_hardware: LinkKind,
    last_role: LinkRow,
    link_choices: HashMap<String, LinkKind>,
    link_loaded: Option<&'static str>,
    link_reload: Option<(String, &'static str)>,
    reload: Option<Reload>,
    link_menu_faces: Vec<(TexId, u32, u32)>,
    link_linked_face: Option<(TexId, u32, u32)>,
    link_step_faces: Vec<(TexId, u32, u32)>,
    link_fail_faces: Vec<(TexId, u32, u32)>,
    link_legend_faces: Vec<(TexId, u32)>,
    starting: Option<LinkStarting>,
    link_transport: Option<(u16, Box<dyn LinkChannel>)>,
    restarting: bool,
    act_at: Millis,
    root: Option<PathBuf>,
    state: SlotState,
    vol_before: Vec<(u8, bool, Millis)>,
    snapshot: Option<Box<dyn Snapshot>>,
    core: Core,
    colour_pending: Option<bool>,
    palette_live: bool,
    palette_pending: Option<GbPalette>,
    link_player: Option<u8>,
    platform: Platform,
    named_core: bool,
    video_mode: VideoMode,
    link: Option<LinkSession>,
    sfx: Option<Sfx>,
    polaroids: Option<Polaroids>,
    pending: Option<(PendingUndo, Millis)>,
    legend_faces: Vec<TexId>,
    undo_face: Option<TexId>,
    clock_faces: Option<(TexId, TexId)>,
    quick_menu_faces: Option<QuickMenuFaces>,
    quick_clock_faces: Option<[(TexId, u32, u32); 2]>,
    sticker_face: Option<TexId>,
    /// The shortcut card's rows, in the order they are written and each with the height it was
    /// rasterised at. Uploaded at boot with the rest of the fixed furniture. The heights are as
    /// much the point as the textures: the card is exactly as tall as its rows make it.
    shortcut_rows: Vec<(TexId, u32, u32)>,
    /// The one line of the card that does not move.
    shortcut_hint: Option<(TexId, u32, u32)>,
    /// One picture from `Wallpapers`, behind everything the shelf draws. `None` on a card
    /// that carries none, which is the common case.
    wallpaper: Option<TexId>,
    battery_percent: slot_ui::Printed,
    bolt: Option<TexId>,
    shelf_platform: slot_ui::Printed,
    name_pending: bool,
    slot_letter: Option<char>,
    shelf_named: Option<Millis>,
    shelf_clock: slot_ui::Printed,
    hud: Hud,
    /// Whether the cart's cheat list (from `System/Cheats/<stem>.txt`) is currently applied to
    /// the core. Toggled by SELECT+A in game; persists across eject/reinsert within a session.
    pub(crate) cheats_on: bool,
    /// The seated cart's cheat codes, each with its own on/off. Loaded from
    /// `System/Cheats/<stem>.txt` when its core spawns; `set_cheats` rebuilds it and shuts the
    /// table. Browsable and switchable one by one in the cheat table (SELECT+A).
    cheats: Vec<CheatItem>,
    /// `Some` while the cheat table overlay is up, holding the row in hand. `None` otherwise.
    cheat_menu: Option<usize>,
    /// One rasterised row face per cheat, in cheat order, (re)built by the frontend when the
    /// seated cart changes. The ON/OFF state is drawn live beside each, so these never change
    /// on a toggle.
    cheat_faces: Vec<(TexId, u32, u32)>,
    /// The stem whose faces `cheat_faces` currently holds, so the frontend re-rasterises only
    /// when the cart under the highlight actually changes.
    cheat_face_stem: Option<String>,
    /// The "no cheats for this cart" panel, rasterised once at boot and shown when the table
    /// opens on a cart that carries none.
    cheat_empty_face: Option<(TexId, u32, u32)>,
    /// Set when a row is toggled inside the table, so `Session` knows to re-push the list to
    /// the core after `apply` returns (the table itself cannot reach the emulator).
    cheats_dirty: bool,
    /// Set when the light/dark mode has moved under the binary's feet. Everything on this side
    /// of the palette — the case, the scrim, the gauge, the bar — follows the mode by itself,
    /// because it is drawn as filled quads. Everything *baked* does not, and this flag is how
    /// the app says so: `Frontend` takes it and re-rasterises.
    mode_dirty: bool,
    /// A machine shelf has been swapped in, so the row's geometry has moved under the cart art,
    /// the shadow and the stand-in — all three were minted for the machine the device came up
    /// on. The frontend takes this and re-cuts them, the way `mode_dirty` re-cuts the furniture.
    shelf_dirty: bool,
    /// How far up the game layer's own screen is. Not a phase: it outlives the insert, since
    /// the cart is home and the chrome is still on screen while the picture arrives.
    screen: f32,
    lift: f32,
    game_ready: bool,
    clock: f64,
    power: Option<Power>,
    dozed_at: Millis,
    woke_on_press: bool,
    autosave_at: Millis,
    pending_save: Option<std::thread::JoinHandle<()>>,
    card_failing: Arc<AtomicBool>,
    card_read_only: bool,
    card_check_at: Millis,
    battery_at: Millis,
    charge_at: Millis,
    headphones: bool,
    headphones_at: Millis,
    battery: Option<Battery>,
    last_led: Option<LedState>,
    /// In-game display filter (panel mask + colour correction), cycled with SELECT+X — which serves
    /// the art on machines that ship it and the aperture grid on the one that does not.
    display: DisplayFilter,
    /// Which page of `palettes::GB_PALETTES` the palette key is walking, and which cell of that
    /// page the browser has under its caret (`None` while the browser is closed).
    ///
    /// The page outlives the browser on purpose: a hold opens the browser, the user turns to the
    /// page they want and lets go, and the tap walks *that* page. Keeping the two in one place is
    /// what makes "the page you were last looking at" true — a second copy of it on the browser
    /// would be a second answer to which fifteen the key is cycling.
    pal_page: usize,
    pal_cursor: Option<usize>,
    /// The names of the entries on the page that is up, one face per cell in table order, as the
    /// frontend rasterised them — **on the machines whose names fit a cell, and empty on the one
    /// whose do not** (see `palette_names`). One list rather than two, because a panel either
    /// names its cells or names the one under the caret, and which of those it is can be read off
    /// this being empty.
    pal_names: Vec<(TexId, u32, u32)>,
    /// The entry under the caret, for the tables whose cells carry no names of their own. The
    /// Game Boy's reading, and the only one it has ever had.
    pal_name: Option<(TexId, u32, u32)>,
    /// The audio latency profile, chosen on the shelf with SELECT+VOL. Read from
    /// `System/audio.txt` at boot and written back whenever it changes.
    audio: Profile,
    /// A per-system screen overlay PNG from the card — `Overlay/GB/GB.png` (plain Game Boy)
    /// or `Overlay/GBC/GBC.png` (Game Boy Color) — drawn over the live game with its own alpha
    /// channel, so the user can theme the screen (bezel, dot-matrix grid, tint) without a
    /// rebuild. `None` = the built-in look. The RGBA is decoded once per insert in
    /// `Session::spawn_core`; the GL texture is minted lazily by the compositor when
    /// `overlay_dirty` is set, and released when the next cart has none.
    overlay_pixels: Option<(Vec<u8>, u32, u32)>,
    /// The compositor's texture handle for the decoded overlay, or `None` when there is no
    /// overlay on screen. Only the compositor can mint or drop the GL resource this names.
    overlay_tex: Option<TexId>,
    /// Set when `overlay_pixels` changed and the texture must be (re)built or released on the
    /// next frame. Pushed by `set_overlay`; cleared by the frontend after `sync_overlay`.
    overlay_dirty: bool,
    /// Every overlay PNG the seated cart's system offers, in rotation order, and which one is
    /// up. Built on insert (`Session::spawn_core`) from `Overlay/`; `overlay_next` cycles it.
    /// Empty for GBA, which has no overlay set.
    overlay_list: Vec<crate::root::OverlayFile>,
    /// Which system `overlay_index` counts in: 0 for GB, 1 for GBC. Kept because a Game Boy's
    /// index counts its three models before the card's own overlays and a Game Boy Color's does
    /// not, so the same number means different things on the two shelves.
    overlay_slot: usize,
    overlay_index: usize,
    /// Which overlay each system last showed, GB then GBC, persisted to `System/overlay.txt` so
    /// the choice survives a boot. Clamped against the current list length when it is applied.
    overlay_indices: [usize; 2],
    /// Which machine's shelf is up. The row holds only this system's carts; SELECT+L2 cycles it.
    shelf_system: System,
    /// Whether the favourites sub-shelf is up within the current system shelf. The whole of what
    /// the bottom-right star reads, and what the letter strip and letter jumps stand down for —
    /// a system shelf is still a library with letters, a favourites shelf is not.
    fav_view: bool,
    /// The three machine logos drawn in the bottom-left corner, in `GB, GBC, GBA` order, each
    /// with the size it was rasterised at. The lit one is chosen by `shelf_system`.
    shelf_ind_faces: Vec<(TexId, u32, u32)>,
    /// The same three badges for the light theme, which the card draws separately. Held as its
    /// own set rather than tinted, because a badge is artwork: the two are chosen between by
    /// the palette's mode, which can change while the device is on.
    shelf_ind_light: Vec<(TexId, u32, u32)>,
    /// The system-shelf swap in flight, if any. While it is `Some` the app takes no input.
    shelf_switch: Option<ShelfSwitch>,
    powering_off: bool,
    radio: Box<dyn RadioJobs>,
    state_writer: crate::state_writer::StateWriter,
}

const FACE_AHEAD: i32 = 8;
const FACE_KEEP: i32 = 12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FaceWant {
    pub shelf: usize,
    pub index: usize,
    pub on_screen: bool,
}

impl App {
    pub fn new(carts: Vec<Cart>) -> Self {
        let letter_counts = tally_letters(&carts);
        let state_writer = crate::state_writer::StateWriter::spawn();
        let card_failing = state_writer.card_failing();
        App {
            radio: radio_jobs(),
            state_writer,
            phase: Phase::Shelf,
            shelf: Shelf::new(carts),
            letters: Letters::new(),
            letter_counts,
            letter_faces: Vec::new(),
            letter_ridge_face: None,
            play_held: None,
            fav_held: None,
            sys_held: None,
            mtp_held: None,
            fav_star: None,
            fav_ind: None,
            refusal: None,
            refused_from: None,
            alert_face: None,
            shelf_title_face: None,
            shutdown_faces: Vec::new(),
            power_menu: None,
            power_menu_faces: Vec::new(),
            direct_link: None,
            direct_faces: Vec::new(),
            direct_at: 0,
            direct_ip: None,
            direct_ip_face: None,
            direct_ip_dirty: false,
            core_picker: None,
            core_board_face: None,
            core_lid_face: None,
            core_faces_stem: None,
            core_socket_faces: Vec::new(),
            core_chip_faces: Vec::new(),
            core_blank_chip_face: None,
            core_chip_shadow_face: None,
            core_legend_faces: Vec::new(),
            game_menu: None,
            link_sprites: None,
            link_hardware: LinkKind::Cable,
            last_role: LinkRow::Host,
            link_choices: HashMap::new(),
            link_loaded: None,
            link_reload: None,
            reload: None,
            link_menu_faces: Vec::new(),
            link_linked_face: None,
            link_step_faces: Vec::new(),
            link_fail_faces: Vec::new(),
            link_legend_faces: Vec::new(),
            starting: None,
            link_transport: None,
            restarting: false,
            act_at: 0,
            root: None,
            state: SlotState::default(),
            vol_before: Vec::new(),
            snapshot: None,
            core: Core::default(),
            colour_pending: None,
            palette_live: false,
            palette_pending: None,
            link_player: None,
            platform: Platform::default(),
            named_core: false,
            video_mode: VideoMode::default(),
            link: None,
            sfx: None,
            polaroids: None,
            pending: None,
            legend_faces: Vec::new(),
            undo_face: None,
            clock_faces: None,
            quick_menu_faces: None,
            quick_clock_faces: None,
            sticker_face: None,
            shortcut_rows: Vec::new(),
            shortcut_hint: None,
            wallpaper: None,
            battery_percent: slot_ui::Printed::default(),
            bolt: None,
            shelf_platform: slot_ui::Printed::default(),
            name_pending: false,
            slot_letter: None,
            shelf_named: None,
            shelf_clock: slot_ui::Printed::default(),
            hud: Hud::new(),
            cheats_on: true,
            cheats: Vec::new(),
            cheat_menu: None,
            cheat_faces: Vec::new(),
            cheat_face_stem: None,
            cheat_empty_face: None,
            cheats_dirty: false,
            mode_dirty: false,
            shelf_dirty: false,
            screen: 0.0,
            lift: 1.0,
            game_ready: false,
            clock: 0.0,
            power: None,
            dozed_at: 0,
            woke_on_press: false,
            autosave_at: AUTOSAVE_MS,
            pending_save: None,
            card_failing,
            card_read_only: false,
            card_check_at: 0,
            battery_at: BATTERY_POLL_MS,
            charge_at: CHARGE_POLL_MS,
            headphones: false,
            headphones_at: 0,
            battery: None,
            last_led: None,
            powering_off: false,
            audio: Profile::default(),
            overlay_pixels: None,
            overlay_tex: None,
            overlay_dirty: false,
            overlay_list: Vec::new(),
            overlay_slot: 0,
            overlay_index: 0,
            overlay_indices: [0, 0],
            shelf_system: System::Gba,
            fav_view: false,
            shelf_ind_faces: Vec::new(),
            shelf_ind_light: Vec::new(),
            shelf_switch: None,
            display: DisplayFilter::new(
                slot_gfx::builtin_panel_mask(),
                DisplayFilter::DEFAULT_CC,
                2,
                [0, 0, 0],
            ),
            // Page one, browser shut. `App::boot` moves the page onto whatever the card had
            // chosen, so the first tap of the palette key walks the fifteen around it.
            pal_page: 0,
            pal_cursor: None,
            pal_names: Vec::new(),
            pal_name: None,
        }
    }

    pub fn boot(root: &Path) -> Self {
        crate::root::ensure(root);
        crate::root::migrate(root);
        // Read the card's label preference (strip_tags) before the first scan, so the displayed
        // and filed names both honour it. A missing file leaves the long-standing default.
        slot_store::init_label_config(root);
        // Before anything is drawn. The card's palette cannot change while the device is on,
        // so it is read once and never asked for again.
        slot_ui::set_theme(Theme::read(root));
        // The card's own cart art, if it has any, also read once before the first frame and
        // before the face builder thread starts. Read once because a cart face is built on that
        // thread and the art has to be there, whole, by then; and said out loud because a card
        // whose art was refused — an index-coloured PNG, say — has nothing else to say so.
        slot_ui::install_cart_art(slot_ui::load_cart_art(root));
        eprintln!("slot: cart art: {}", slot_ui::cart_art_summary());
        crate::boot_time::mark("root and theme");
        // The cached scan, not the plain one: a boot re-reads only the carts that changed
        // since the last one, which on a card of a few hundred games is the difference
        // between a shelf and a wait. See `slot_store::scan_cached`.
        let carts = scan_cached(root).unwrap_or_default();
        crate::boot_time::mark("scan");
        let mut app = App::new(carts);
        app.root = Some(root.to_path_buf());
        // The display filter (panel mask + colour correction) is read from the card so the
        // shipping look can be a per-card choice, then cycled live during play. The mask falls
        // back to the built-in LCD3x table; the colour correction falls back to the grayscale (0%
        // default; the modes fall back to (2, 0) (LCD3X + OFF, the shipped look).
        let lcd3x = crate::root::panel_mask(root).unwrap_or_else(slot_gfx::builtin_panel_mask);
        let nocolor_cc = crate::root::color_correction(root).unwrap_or(DisplayFilter::DEFAULT_CC);
        // Before the modes are read, not after: a card written before the Game Boy's palette list
        // was the table it is now has an index into the *old* one, and half of that range means
        // something else here. `migrate_palette` puts the look back where it was and leaves a
        // marker so it happens exactly once per card.
        crate::root::migrate_palette(root);
        crate::root::migrate_display_modes(root);
        let (mode, cc) = crate::root::display_modes(root);
        app.display = DisplayFilter::new(lcd3x, nocolor_cc, mode, cc);
        // The page the palette key walks starts on the entry the card left the Game Boy at, so
        // the first tap continues from that palette rather than from page one.
        app.pal_page = app.gb_palette_index() / slot_ui::PALETTE_PER_PAGE;
        app.audio = crate::root::audio_profile(root);
        app.state = read_slot_state(root);
        // Which machine's shelf opens, and which overlay each system was last showing, both
        // off the card. A cart still in the slot outranks the stored shelf — a resume has to
        // open the shelf that holds it — and a stored shelf the card has no carts for is
        // skipped, so a card whose only games are, say, GB never opens on an empty GBA shelf.
        app.overlay_indices = crate::root::overlay_indices(root);
        let last_system = app
            .state
            .cart
            .as_deref()
            .and_then(|stem| app.shelf.carts.iter().find(|c| c.stem == stem))
            .map(|c| c.system());
        let mut system = last_system.unwrap_or_else(|| crate::root::shelf_system(root));
        if !app.shelf.carts.iter().any(|c| c.system() == system) {
            system = [System::Gba, System::Gb, System::Gbc]
                .into_iter()
                .find(|s| app.shelf.carts.iter().any(|c| c.system() == *s))
                .unwrap_or(System::Gba);
        }
        app.shelf_system = system;
        app.shelf.set_system(system);
        let view = app.system_view(system);
        app.shelf.set_view(view, None);
        app.retally_letters();
        // The palette, before anything has been rasterised: every face on the device is baked
        // with the ink burned into it, so the mode has to be in place before the first one, not
        // after. Anything later than this line is a light-mode device drawn in dark type.
        slot_ui::palette::set_mode(app.state.mode);
        if app.state.clock_set {
            app.start();
        } else {
            app.phase = clock_screen(system_secs(), 0, false);
        }
        app
    }

    /// The mask the game pass multiplies by, resolved through the current aperture preset.
    /// Neutral on the two machines with a screen bezel: their lattice is drawn in the game shader
    /// (`display_grid`) — which is also why it can be *white*, since a multiply table can only
    /// ever darken. The Advance, which has no overlay to hang a mesh off, is on the table.
    pub fn display_mask(&self) -> [[[u8; 3]; 3]; 3] {
        self.display
            .applied_mask(matches!(self.current_machine(), System::Gb | System::Gbc))
    }

    /// The Game Boy's four shades right now, whichever of the two tables they came from: a model's
    /// own set while a model's screen art is up, and the entry `cc` holds otherwise.
    ///
    /// This is the one place that decides which of the two wins, so the palette, the lattice and
    /// the boot log cannot disagree about it.
    fn gb_shades(&self) -> Option<[[u8; 3]; 4]> {
        if self.current_machine() != System::Gb {
            return None;
        }
        if let Some(m) = App::model_of_index(self.overlay_slot, self.overlay_index) {
            let m = m.min(crate::palettes::GB_MODELS.len() - 1);
            return Some(crate::palettes::GB_MODELS[m].1);
        }
        Some(crate::palettes::GB_PALETTES[self.gb_palette_index()].1)
    }

    /// The Game Boy's entry in the palette table, clamped to it. Out of range can only come from
    /// a `display.txt` edited by hand, and a card that names entry 9000 should show the last
    /// palette rather than have the app fall over.
    fn gb_palette_index(&self) -> usize {
        (self.display.cc[DisplayFilter::machine_slot(System::Gb)] as usize)
            .min(crate::palettes::GB_PALETTES.len() - 1)
    }

    /// The panel lattice for the machine that is up, as its colour and how much of it to mix.
    ///
    /// A Game Boy's colour is the current palette's **lightest shade**, which is the background:
    /// a DMG's mesh is olive, a Light's is cyan, and neither is written down anywhere — change the
    /// palette and the lattice follows.
    ///
    /// A Game Boy Color has no four shades to take a colour from, so its lattice is plain — white
    /// or black — and unlike the Game Boy's, which is always on, the Color's is **switched by the
    /// overlay ring** rather than by a key of its own: every overlay appears once per grid state,
    /// and the state is the overlay index modulo `GBC_GRID_STATES` (see `overlay_next`). This is
    /// why the colour machines need no aperture table: the mesh and the bezel are one choice.
    ///
    /// An Advance is on the aperture table instead (`display_mask`), so it draws no mesh at all:
    /// this is the two bezel machines' lattice and nothing else.
    ///
    /// The third element is the scanline flag: `true` keeps the bottom row alone.
    pub fn display_grid(&self) -> Option<([f32; 3], f32, bool)> {
        match self.current_machine() {
            System::Gb => {
                let light = self.gb_shades()?[0];
                Some((
                    [
                        light[0] as f32 / 255.0,
                        light[1] as f32 / 255.0,
                        light[2] as f32 / 255.0,
                    ],
                    DisplayFilter::GB_GRID_MIX,
                    false,
                ))
            }
            System::Gbc => {
                if self.overlay_slot != 1 {
                    return None;
                }
                let colour = match self.overlay_index % DisplayFilter::GBC_GRID_STATES {
                    1 => [1.0, 1.0, 1.0],
                    2 => [0.0, 0.0, 0.0],
                    _ => return None,
                };
                Some((colour, DisplayFilter::GBC_GRID_MIX, false))
            }
            System::Gba => None,
        }
    }

    /// Which machine the display filter is being asked about: the cart under the caret, which
    /// during play is the cart in the slot, and the shelf's own machine when there is no caret
    /// (an empty card). GB and GBC are two machines with two palettes even though they share one
    /// screen geometry, so this cannot be `screen_system`.
    pub fn current_machine(&self) -> System {
        self.shelf
            .current_cart()
            .map(|c| c.system())
            .unwrap_or(self.shelf_system)
    }

    /// The colour-correction matrix for the game pass right now, identity when off.
    pub fn display_cc(&self) -> [[f32; 3]; 3] {
        self.display.applied_cc(self.current_machine()).0
    }

    /// ...and its black point, which is the whole difference between a saturation grade and a
    /// hardware palette. Zero now that the Game Boy is drawn by a lookup and not a matrix; the
    /// colour machines never had one.
    pub fn display_cc_bias(&self) -> [f32; 3] {
        self.display.applied_cc(self.current_machine()).1
    }

    /// The Game Boy's palette as a 256-entry RGBA lookup, or `None` when this frame is not a
    /// Game Boy palette at all — 原生灰, where the core's own greys are the look, and every
    /// colour machine, which uses the matrix above.
    ///
    /// Worth being clear about the cost, because it looks like the expensive one and is not:
    /// the matrix path is two `pow`s and nine multiplies per pixel, and this is one extra
    /// nearest-neighbour sample replacing all of it. `pow` is the instruction to avoid on this
    /// class of GPU.
    pub fn display_palette(&self) -> Option<[u8; 1024]> {
        let machine = self.current_machine();
        if machine != System::Gb {
            // A colour machine looks through a ramp only when its grade is a duotone: everything
            // else down the list is a matrix or a baked cube, which have their own paths.
            let mode = self.display.cc[DisplayFilter::machine_slot(machine)] as usize;
            return match GRADES[mode.min(GRADES.len() - 1)].kind {
                GradeKind::Ramp(dark, light) => Some(ramp_lut(dark, light)),
                _ => None,
            };
        }
        // 原生灰 is the core's own greys, so there is nothing to look up — and only the *table's*
        // own entry means that. A model is always a palette: its four shades are the other half of
        // the art, not a preference layered over it.
        if self.display.cc[DisplayFilter::machine_slot(machine)] as usize
            == crate::palettes::GB_NEUTRAL
            && App::model_of_index(self.overlay_slot, self.overlay_index).is_none()
        {
            return None;
        }
        Some(DisplayFilter::gb_lut(self.gb_shades()?))
    }

    /// What the Game Boy's current palette is called, for the boot log. One table, names and
    /// colours together, so a name cannot drift from what it names.
    pub fn display_palette_name(&self) -> &'static str {
        if self.current_machine() == System::Gb {
            if let Some(m) = App::model_of_index(self.overlay_slot, self.overlay_index) {
                return crate::palettes::GB_MODEL_NAMES
                    [m.min(crate::palettes::GB_MODEL_NAMES.len() - 1)];
            }
        }
        crate::palettes::GB_PALETTES[self.gb_palette_index()].0
    }

    /// 当前色彩校正该用的 gamma：灰度(0) 保持 1.0（编码空间直乘，黑白不压暗）；
    /// AGB-001(1)/NDS(2) 走线性空间，AGB-001 用 2.2（更暗）、NDS 用 2.0（稍亮）。
    /// GB 的色板同理：原生灰走 1.0（本来就是核心给的灰），其余色板都走 `CC_GAMMA`
    /// —— LUT 的插值要在线性空间里做，否则中间两档会被压暗。
    pub fn display_cc_gamma(&self) -> f32 {
        let machine = self.current_machine();
        let mode = self.display.cc[DisplayFilter::machine_slot(machine)] as usize;
        if machine == System::Gb {
            if mode == crate::palettes::GB_NEUTRAL
                && App::model_of_index(self.overlay_slot, self.overlay_index).is_none()
            {
                return 1.0;
            }
            return DisplayFilter::CC_GAMMA;
        }
        match GRADES[(mode as usize).min(GRADES.len() - 1)].kind {
            GradeKind::Matrix(_, g) => g,
            // A ramp or a baked cube is drawn by a lookup, and the lookup is built in the space
            // the shader samples it in — so there is no linearisation to ask for.
            _ => 1.0,
        }
    }

    /// The pixel-art lookup when the pixel grade is in force on a colour machine, else `None`.
    /// `frontend` pushes it every frame, the same way it pushes the Game Boy's palette.
    pub fn display_pixel_lut(&self) -> Option<&[u8]> {
        let machine = self.current_machine();
        if machine == System::Gb {
            return None;
        }
        let mode = self.display.cc[DisplayFilter::machine_slot(machine)] as usize;
        match GRADES[mode.min(GRADES.len() - 1)].kind {
            GradeKind::Lut(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The audio profile the shelf has settled on. `Session::sync_audio_profile` is what
    /// notices a change and reopens the device for it.
    pub fn audio_profile(&self) -> Profile {
        self.audio
    }

    /// SELECT+VOL+ / SELECT+VOL- on the shelf: step the audio profile and persist it to
    /// `System/audio.txt`. Nothing is reopened here — the sink belongs to the session, and
    /// this only decides what it should open for.
    fn cycle_audio(&mut self, forward: bool) {
        self.audio = if forward {
            self.audio.next()
        } else {
            self.audio.prev()
        };
        if let Some(root) = &self.root {
            crate::root::write_audio_profile(root, self.audio);
        }
        // The profile moves no pixel of the picture, so this line is the only thing that tells
        // the player the press landed at all.
        let said = match self.audio {
            Profile::Stable => Toast::AudioStable,
            Profile::Balanced => Toast::AudioBalanced,
            Profile::Strict => Toast::AudioStrict,
        };
        self.hud.toast(said, self.now());
    }

    /// SELECT+X on an Advance: step the aperture table through its `MASK_STATES` presets (off ->
    /// LCD3X at 25, 50, 75 and 100% -> SCANLINE at 50 and 100%) and persist the choice to
    /// `System/display.txt`.
    fn cycle_mask(&mut self) {
        self.display.cycle_mask();
        if let Some(root) = &self.root {
            crate::root::write_display_modes(root, self.display.mask_mode, self.display.cc);
        }
    }

    /// SELECT+Y: advance the colour correction of the machine that is up, and persist all of
    /// them to `System/display.txt`. On a colour machine that is grayscale -> AGB-001 -> NDS;
    /// on a Game Boy it is the next palette on the page the browser last showed.
    ///
    /// **One page, not the whole table.** The Game Boy's list is every palette gambatte ships —
    /// hundreds of them — and a key that walked all of them would be a key nobody could aim. The
    /// page is the unit: hold the key to open the browser and turn to the page you want, let go,
    /// and this walks the fifteen on it. That is also what makes the ring's length something the
    /// user can see, rather than a number that changes with a palette pack.
    ///
    /// **Refused while a model is up**, and that is the whole of what "bound" means. A model's
    /// four shades are not a preference layered over its art, they are the other half of it: the
    /// DMG screen was drawn against the DMG ramp, and letting this key recolour the picture
    /// inside that case would leave the two disagreeing with nothing on screen to say so. The
    /// press is refused rather than dropped, so it reads as answered — the same as every other
    /// "nothing doing" action in this file.
    fn cycle_cc(&mut self) {
        if self.current_machine() == System::Gb {
            if App::model_of_index(self.overlay_slot, self.overlay_index).is_some() {
                self.refuse();
                return;
            }
            self.step_palette();
        } else {
            // A colour machine's list is two pages of nine, and the tap walks **the page the
            // grade in hand is on** — so `pal_page` is not consulted at all. It counts pages of
            // the Game Boy's hundreds-long palette table (`boot` seeds it from the GB slot), and
            // reading it here put the walk's base far past the end of an eighteen-entry list:
            // `cycle_on_page` then answered the last entry for ever, which is exactly the "the
            // key works once" the user saw. Deriving the page from the index keeps the two
            // tables apart and needs no second piece of state.
            let machine = self.current_machine();
            let slot = DisplayFilter::machine_slot(machine);
            let per = App::grades_per_page(machine);
            let at = self.display.cc[slot] as usize;
            let page = (at / per).min(GRADES.len().div_ceil(per).saturating_sub(1));
            let next = slot_store::cycle_on_page(page, at, GRADES.len(), per);
            self.display.cc[slot] = next as u16;
        }
        if let Some(root) = &self.root {
            crate::root::write_display_modes(root, self.display.mask_mode, self.display.cc);
        }
    }

    /// How many grades one page of the colour browser shows. The Game Boy's table is fifteen to a
    /// page (three columns of five); a colour machine's list is two pages of nine, three by three.
    pub fn grades_per_page(machine: System) -> usize {
        if machine == System::Gb {
            slot_ui::PALETTE_PER_PAGE
        } else {
            9
        }
    }

    /// The Game Boy's palette one along the page that is up, wrapping at the end of that page.
    /// `self.pal_page` is the page the browser was last showing, so a palette picked on page 7
    /// keeps being cycled among page 7's fifteen until the browser is opened again.
    fn step_palette(&mut self) {
        let total = crate::palettes::GB_PALETTES.len();
        let next = slot_store::cycle_on_page(
            self.pal_page,
            self.gb_palette_index(),
            total,
            slot_ui::PALETTE_PER_PAGE,
        );
        self.display.cc[DisplayFilter::machine_slot(System::Gb)] = next as u16;
    }

    /// Into the slot or onto the shelf. Reached on boot once the clock is known, and from
    /// the clock screen when it becomes known.
    fn start(&mut self) {
        // One cart is a dedicated device. There is nothing to choose between, so whatever
        // `slot.state` remembers, including a cart that is no longer on the card, names the
        // only thing it could have meant.
        // A position in the *view*, not the library: the shelf is one machine's carts, and a
        // library index would seat the caret on the wrong cart — or past the end of a shorter
        // view — the moment the shelf is filtered. `index_of_stem` reads the view.
        let seated = if self.single_cart() {
            Some(0)
        } else {
            self.state
                .cart
                .as_deref()
                .and_then(|stem| self.shelf.index_of_stem(stem))
        };
        self.phase = Phase::Shelf;
        match seated {
            Some(i) => {
                self.shelf.index = i;
                self.shelf.scroll = i as f32;
                self.insert(false);
                if let Phase::Inserting { resumed, t, .. } = &mut self.phase {
                    *resumed = true;
                    *t = INSERT_S;
                }
            }
            None => {
                self.state.cart = None;
                self.state.cart_platform = None;
                self.name_pending = true;
            }
        }
        // The strip starts on the letter of whatever the row is showing. Snapped rather than
        // travelled to: there is no previous position at a boot, and a strip arriving from `#`
        // would be the shelf's first movement being one nobody made.
        if let Some(cart) = self.shelf.current_cart() {
            self.letters.snap_to(cart.initial);
        }
    }

    fn shelf(&self) -> &Shelf {
        &self.shelf
    }

    fn shelf_mut(&mut self) -> &mut Shelf {
        &mut self.shelf
    }

    fn jump_letter(&mut self, dir: i32) {
        let before = self.shelf().index;
        match dir > 0 {
            true => self.shelf_mut().jump_next_letter(),
            false => self.shelf_mut().jump_prev_letter(),
        }
        if self.shelf().index == before {
            return;
        }
        let Some(letter) = self.selected_stem().map(slot_store::initial) else {
            return;
        };
        if self.slot_letter != Some(letter) {
            self.shelf_platform = slot_ui::Printed::default();
        }
        self.slot_letter = Some(letter);
        self.shelf_named = None;
        self.name_pending = true;
    }

    fn slot_name_alpha(&self) -> f32 {
        let Some(at) = self.shelf_named else {
            return 0.0;
        };
        let t = self.now().saturating_sub(at);
        let level = if t < SLOT_NAME_IN_MS {
            t as f32 / SLOT_NAME_IN_MS as f32
        } else if t < SLOT_NAME_IN_MS + SLOT_NAME_HOLD_MS {
            1.0
        } else {
            let out = t - SLOT_NAME_IN_MS - SLOT_NAME_HOLD_MS;
            1.0 - (out as f32 / SLOT_NAME_OUT_MS as f32).min(1.0)
        };
        SLOT_NAME_ALPHA * ease(level)
    }


    pub fn confirm_clock(&mut self) {
        let Phase::SetClock {
            picker,
            seed,
            from_menu,
        } = &self.phase
        else {
            return;
        };
        let (moved, offset, from_menu) = (picker.secs() - *seed, picker.offset_min(), *from_menu);
        let utc = self.utc_secs() + moved;
        if let Some(power) = &mut self.power {
            power.set_clock(utc);
        }
        self.state.utc_offset_min = offset as i16;
        self.state.clock_set = true;
        self.persist();
        if from_menu {
            self.phase = Phase::QuickMenu {
                row: QuickRow::DateTime,
            };
        } else {
            self.start();
        }
    }

    /// Seconds since the epoch, from the platform once there is one. The shelf clock and the
    /// polaroid captions both read it, so neither can disagree with the cartridge RTC.
    /// Where the card is mounted. `None` only in the tests that never touch one.
    /// Which machine's shelf is up. The cart's own size on screen follows from it, so anything
    /// drawing a row of carts has to ask rather than assume the Advance cart's.
    pub fn shelf_system(&self) -> slot_store::System {
        self.shelf_system
    }

    /// Where the shelf's line of type sits: centred in the gap between the row's own foot and
    /// the top of the case. Derived rather than picked, so moving either end — a taller cart, a
    /// deeper bay — carries it along instead of leaving it behind.
    fn shelf_title_y(&self) -> f32 {
        let foot = slot_ui::size_for(self.shelf_system).foot_y();
        let case = OUT_H as f32 - MOUTH_H;
        (foot + case) / 2.0 - SHELF_TITLE_H as f32 / 2.0
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn wall_secs(&self) -> i64 {
        self.utc_secs() + i64::from(self.state.utc_offset_min) * 60
    }

    fn utc_secs(&self) -> i64 {
        self.power.as_ref().map_or_else(system_secs, |p| p.now())
    }

    pub fn picker(&self) -> Option<&ClockPicker> {
        match &self.phase {
            Phase::SetClock { picker, .. } => Some(picker),
            _ => None,
        }
    }

    pub fn quick_menu(&self) -> Option<QuickRow> {
        match self.phase {
            Phase::QuickMenu { row } => Some(row),
            _ => None,
        }
    }

    pub fn quick_value(&self, row: QuickRow) -> Option<QuickValue> {
        match row {
            QuickRow::FastForward => QuickValue::speed(self.state.ff_speed),
            QuickRow::FastForwardSound => Some(QuickValue::flag(self.state.ff_sound)),
            QuickRow::ColourCorrection => Some(QuickValue::flag(self.state.colour_correction)),
            QuickRow::GbPalettes => Some(QuickValue::flag(self.state.gb_palettes)),
            QuickRow::GbaShader => Some(shader_value(self.state.shader_gba)),
            QuickRow::GbShader => Some(shader_value(self.state.shader_gb)),
            QuickRow::Rumble => Some(QuickValue::flag(self.state.rumble)),
            QuickRow::EjectSave => Some(QuickValue::flag(self.state.eject_save)),
            QuickRow::Turbo => Some(QuickValue::flag(self.state.turbo)),
            QuickRow::Rewind => Some(QuickValue::flag(self.state.rewind)),
            QuickRow::DateTime | QuickRow::About | QuickRow::Screen | QuickRow::Game => None,
        }
    }

    pub fn set_quick_menu_faces(&mut self, faces: QuickMenuFaces) {
        self.quick_menu_faces = Some(faces);
    }

    pub fn set_quick_clock_faces(&mut self, dim: (TexId, u32, u32), lit: (TexId, u32, u32)) {
        self.quick_clock_faces = Some([dim, lit]);
    }

    pub fn set_sticker_face(&mut self, face: TexId) {
        self.sticker_face = Some(face);
    }

    /// The shortcut card: one face per row and the line that stays at its foot. Fixed strings,
    /// so they are built once at boot and never asked for again.
    pub fn set_shortcut_faces(
        &mut self,
        rows: Vec<(TexId, u32, u32)>,
        hint: Option<(TexId, u32, u32)>,
    ) {
        self.shortcut_rows = rows;
        self.shortcut_hint = hint;
    }

    /// How far the about card can be scrolled: how much taller than the panel its label and its
    /// rows make it. Zero while the rows are not up yet, which is a card that does not move
    /// rather than one that scrolls into nothing.
    fn about_scroll_max(&self) -> f32 {
        let rows: f32 = self.shortcut_rows.iter().map(|(_, _, h)| *h as f32).sum();
        // The last row has to come to rest clear of the pinned hint's band, not under it, so the
        // room below the rows is the band's whole height. Falls back to `ABOUT_PAD` before the
        // hint is up, which is a card that does not move rather than one that scrolls into
        // nothing.
        let foot = self
            .shortcut_hint
            .map(|(_, _, h)| h as f32)
            .unwrap_or(ABOUT_PAD);
        let end = about_first_y() + rows + foot;
        (end - OUT_H as f32).max(0.0)
    }

    pub fn set_clock_faces(&mut self, line: TexId, hint: TexId) {
        self.clock_faces = Some((line, hint));
    }

    pub fn set_cart_shadow(&mut self, face: TexId) -> Option<TexId> {
        self.shelf.set_shadow(face)
    }

    /// The blank cart a cart whose own face is not built yet is drawn as. Uploaded at boot,
    /// with the shadow: it is the same for every cart and never changes — except when the row's
    /// machine does, which is why this hands back what it replaced.
    pub fn set_cart_placeholder(&mut self, face: TexId) -> Option<TexId> {
        self.shelf.set_placeholder(face)
    }

    pub fn set_gb_cart_shadows(&mut self, notched: TexId, rounded: TexId) {
        self.shelf.set_gb_shadow(GbShell::Notched, notched);
        self.shelf.set_gb_shadow(GbShell::Rounded, rounded);
    }

    pub fn set_wallpaper(&mut self, face: TexId) {
        self.wallpaper = Some(face);
    }

    pub fn set_bolt_face(&mut self, bolt: TexId) {
        self.bolt = Some(bolt);
    }

    pub fn set_shelf_platform_face(&mut self, face: TexId, w: u32) {
        self.shelf_platform = slot_ui::Printed::new(face, w);
    }

    pub fn clear_shelf_platform(&mut self) {
        self.shelf_platform = slot_ui::Printed::default();
    }

    pub fn slot_text(&self) -> Option<String> {
        match self.slot_letter {
            Some(letter) => Some(letter.to_string()),
            None => self.shelf_platform_name().map(str::to_string),
        }
    }

    pub fn shelf_platform_name(&self) -> Option<&'static str> {
        let other = [System::Gba, System::Gb, System::Gbc].into_iter().any(|s| {
            s != self.shelf_system && self.shelf.carts.iter().any(|c| c.system() == s)
        });
        if !other {
            return None;
        }
        Some(match self.shelf_system {
            System::Gba => "Game Boy Advance",
            System::Gb => "Game Boy",
            System::Gbc => "Game Boy Color",
        })
    }

    pub fn set_battery_percent_face(&mut self, face: TexId, w: u32) {
        self.battery_percent = slot_ui::Printed::new(face, w);
    }

    pub fn set_shelf_clock_face(&mut self, face: TexId, w: u32) {
        self.shelf_clock = slot_ui::Printed::new(face, w);
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn carts(&self) -> &[Cart] {
        &self.shelf.carts
    }

    pub fn seated_cart(&self) -> Option<&Cart> {
        let stem = match &self.phase {
            Phase::Inserting { cart, .. }
            | Phase::Playing { cart }
            | Phase::Ejecting { cart, .. }
            | Phase::Polaroids { cart } => cart,
            Phase::Doze { cart: Some(cart) } => cart,
            _ => return None,
        };
        self.shelf().carts.iter().find(|c| c.stem == *stem)
    }

    /// Which cart the caret is on. Read by the boot path to decide which faces are worth
    /// rasterising before the first frame, and by the background filler to order the rest.
    pub fn shelf_index(&self) -> usize {
        self.shelf.current().unwrap_or(0)
    }

    /// The carts the row is drawing, as library indices — what a build has to cover, as opposed
    /// to a contiguous run of indices around the caret. See `Shelf::draw_window`.
    pub fn shelf_draw_window(&self) -> Vec<usize> {
        self.shelf.draw_window()
    }

    /// How many carts the shelf's view holds, as opposed to how many are on the card.
    pub fn shelf_view_len(&self) -> usize {
        self.shelf.view_len()
    }

    /// How far a cart is from the caret that would be showing it — the row on screen when the
    /// view holds it, otherwise the front of its own machine's shelf, where a switch lands.
    /// What the resident window and the filler both measure in. See `Shelf::shelf_distance`.
    pub fn shelf_distance(&self, cart: usize) -> Option<usize> {
        self.shelf.shelf_distance(cart)
    }

    /// The cart index in each slot the row would draw, in slot order — the draw's own answer to
    /// "which cart is this", read back for the boot log rather than recomputed.
    pub fn shelf_slot_carts(&self) -> Vec<usize> {
        self.shelf
            .slot_rects(None, 0.0, 0.0)
            .iter()
            .map(|r| r.cart)
            .collect()
    }

    /// Exactly one cart on the card. The shelf is unreachable and eject is refused.
    pub fn single_cart(&self) -> bool {
        self.carts().len() == 1
    }

    /// Face textures in `carts` order; `None` where the face is still being rasterised.
    /// Only the compositor can mint a `TexId`.
    pub fn set_faces(&mut self, faces: Vec<Option<TexId>>) {
        self.shelf.set_faces(faces);
    }

    /// One face, uploaded after `set_faces`, as the background filler finishes it.
    pub fn set_face(&mut self, i: usize, tex: TexId) {
        self.shelf.set_face(i, Some(tex));
    }

    /// Forgets a face whose texture has been released.
    pub fn clear_face(&mut self, i: usize) {
        self.shelf.clear_face(i);
    }

    /// Which face texture a cart is currently holding, if any.
    pub fn face_of(&self, i: usize) -> Option<TexId> {
        self.shelf.face_of(i)
    }

    /// Drop the overlay the previous cart left on screen, for a cart whose system has no overlay
    /// set (GBA) or a card carrying no art for its system. `overlay_pixels` becomes `None` and
    /// `overlay_dirty` is set, so the frontend releases the GL texture and the built-in look
    /// stands. Used in place of `set_overlay_set` for GBA: GBA would otherwise be read as
    /// `slot` 0 (GB) and re-decode a Game Boy model overlay onto a GBA game.
    pub(crate) fn clear_overlay_set(&mut self) {
        self.overlay_pixels = None;
        self.overlay_dirty = true;
    }

    /// Hand the seated cart's overlay set to the app, from `Session::spawn_core`: every
    /// `Overlay/<sys>.png` / `<sys>-NN.png` for the cart's system, in rotation order (`slot` 0
    /// for GB, 1 for GBC), with the persisted index for that system applied. The chosen file is
    /// decoded here and `overlay_dirty` is set so the frontend re-mints the GL texture once.
    /// GBA has no overlay set at all and is routed through `clear_overlay_set` instead.
    pub(crate) fn set_overlay_set(&mut self, list: Vec<crate::root::OverlayFile>, slot: usize) {
        self.overlay_list = list;
        let slot = slot.min(1);
        self.overlay_slot = slot;
        let choices = Self::overlay_choices(slot, self.overlay_list.len());
        self.overlay_index = if choices == 0 {
            0
        } else {
            self.overlay_indices[slot].min(choices - 1)
        };
        self.refresh_overlay_pixels();
    }

    /// How many overlay choices a system has. A Game Boy's three models come first and the card's
    /// own overlays after them; a Game Boy Color offers each of its overlays once per grid state
    /// (without, white, black — `GBC_GRID_STATES` of them), so the mesh is switched by walking the
    /// ring rather than by a key of its own.
    fn overlay_choices(slot: usize, ordinary: usize) -> usize {
        if slot == 0 {
            DisplayFilter::GB_MODEL_COUNT + ordinary
        } else {
            ordinary * DisplayFilter::GBC_GRID_STATES
        }
    }

    /// Which model a combined index names, if it names one.
    fn model_of_index(slot: usize, index: usize) -> Option<usize> {
        (slot == 0 && index < DisplayFilter::GB_MODEL_COUNT).then_some(index)
    }

    /// Where the card's *own* overlay for this index is, if the index is one of those rather
    /// than a model.
    fn ordinary_overlay_at(&self) -> Option<&crate::root::OverlayFile> {
        let at = if self.overlay_slot == 0 {
            self.overlay_index.saturating_sub(DisplayFilter::GB_MODEL_COUNT)
        } else {
            // A Game Boy Color's ring carries each overlay's grid states in a row, so the overlay
            // is one entry in `GBC_GRID_STATES` — see `overlay_choices`.
            self.overlay_index / DisplayFilter::GBC_GRID_STATES
        };
        self.overlay_list.get(at)
    }

    /// Re-decode the chosen overlay file into `overlay_pixels` and mark the texture dirty.
    /// `None` pixels (`None` from a missing/corrupt file, or an empty list) is a real answer:
    /// the frontend reads it as "release the texture", so the built-in look stands.
    fn refresh_overlay_pixels(&mut self) {
        // A model's art is not in `overlay_list` — `overlay_files` cannot name it and should not
        // — so the model index is resolved to its own file here.
        self.overlay_pixels = match Self::model_of_index(self.overlay_slot, self.overlay_index) {
            Some(m) => self
                .root
                .clone()
                .and_then(|root| {
                    crate::root::model_overlay(
                        &root,
                        "gb",
                        crate::palettes::GB_MODELS[m.min(crate::palettes::GB_MODELS.len() - 1)].0,
                    )
                })
                .and_then(|path| slot_ui::decode_rgba(&path)),
            None => self
                .ordinary_overlay_at()
                .and_then(|overlay| slot_ui::decode_rgba(&overlay.path)),
        };
        self.overlay_dirty = true;
    }

    /// Whether the overlay up asks for the screen reflection — its file name carried the `x`
    /// marker. Read by the frontend to hand the compositor the overlay texture as the effect's
    /// zone mask; `None`/false leaves the game drawn exactly as before.
    pub(crate) fn overlay_reflects(&self) -> bool {
        // Every model mask is a reflection layer: that is what they were drawn as, and it is why
        // their names carry the same trailing `x` the ordinary ones use to say so.
        if Self::model_of_index(self.overlay_slot, self.overlay_index).is_some() {
            return true;
        }
        self.ordinary_overlay_at().is_some_and(|o| o.reflect)
    }

    /// The overlay texture the compositor minted, if any. The reflection samples its alpha as
    /// the zone, so the same texture is both the art and the effect's mask.
    pub(crate) fn overlay_tex(&self) -> Option<TexId> {
        self.overlay_tex
    }

    /// Step to the next overlay for the highlighted cart's system (the one in the slot while a
    /// game is playing). Reached from SELECT+X on a Game Boy and a Game Boy Color, and from
    /// SELECT+R2 as well on the Game Boy.
    ///
    /// A Game Boy Color's ring is **`GBC_GRID_STATES` times as long as its art**: every overlay is
    /// offered without the grid, with a white one, and with a black one, so the mesh is a set of
    /// further states of each overlay rather than a setting of its own. The choice is therefore an
    /// index into that ring, and both the art and the grid are read back out of it — see
    /// `ordinary_overlay_at` and `display_grid`.
    ///
    /// A system with a single file has nothing to rotate, and an Advance has no overlay set at
    /// all; both are silent no-ops rather than a refusal, since the press is a step through a
    /// ring and a card may simply carry one image.
    fn overlay_next(&mut self) {
        let Some(system) = self.shelf.current_cart().map(|c| c.system()) else {
            return;
        };
        let slot = match system {
            System::Gb => 0,
            System::Gbc => 1,
            System::Gba => return,
        };
        let Some(root) = self.root.clone() else {
            return;
        };
        let which = if slot == 0 { "gb" } else { "gbc" };
        let list = crate::root::overlay_files(&root, which);
        // A Game Boy's ring is its three models and then the card's own overlays; a Game Boy
        // Color's is just its overlays.
        let choices = Self::overlay_choices(slot, list.len());
        if choices <= 1 {
            return;
        }
        // Note the ring is never empty on a Game Boy even when the card carries no art at all:
        // `choices` counts the three models before anything in `Overlay/`.
        self.overlay_indices[slot] = (self.overlay_indices[slot] + 1) % choices;
        crate::root::write_overlay_indices(&root, self.overlay_indices);
        // Landing on a model adopts its palette — and nothing is written for that here, because
        // there is nothing to write: a model's four shades are not an entry in the table and are
        // not reachable from the palette key, so `gb_shades` reads them straight off the model
        // that is up. The palette the card holds is left exactly as it was, which is what makes
        // walking through the DMG and out again land back on the look that was chosen before it.
        // With the cart in the slot, swap the live set too so the change is on screen now.
        if matches!(self.phase, Phase::Playing { .. }) {
            self.overlay_list = list;
            self.overlay_slot = slot;
            self.overlay_index = self.overlay_indices[slot];
            self.refresh_overlay_pixels();
        }
    }

    /// `true` while the overlay texture needs (re)building. Read by the frontend's `sync_overlay`.
    pub(crate) fn overlay_dirty(&self) -> bool {
        self.overlay_dirty
    }

    /// The decoded overlay pixels, if any — read by `sync_overlay` to (re)build the texture.
    pub(crate) fn overlay_pixels(&self) -> Option<&(Vec<u8>, u32, u32)> {
        self.overlay_pixels.as_ref()
    }

    /// Drop the current overlay texture handle, returning it for the compositor to release.
    pub(crate) fn take_overlay_tex(&mut self) -> Option<TexId> {
        self.overlay_tex.take()
    }

    /// Store the freshly minted overlay texture handle (or `None` when there is no overlay).
    pub(crate) fn set_overlay_tex(&mut self, tex: Option<TexId>) {
        self.overlay_tex = tex;
    }

    /// Clear the dirty flag once `sync_overlay` has reconciled the texture with `overlay_pixels`.
    pub(crate) fn set_overlay_clean(&mut self) {
        self.overlay_dirty = false;
    }

    /// Whether the app is taking no input right now — true only for the length of a
    /// system-shelf swap. `Session::act` reads it so every key is inert mid-swap, the volume
    /// and mute included: they are handled on the session's side of `apply`.
    pub fn input_locked(&self) -> bool {
        self.shelf_switch.is_some()
    }

    /// The three machine logos for the bottom-left indicator, in `GB, GBC, GBA` order, each
    /// with the size it was rasterised at. Uploaded by the frontend at boot.
    pub fn set_shelf_ind_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.shelf_ind_faces = faces;
    }

    /// The light theme's own three, used when the palette is in light mode.
    pub fn set_shelf_ind_light(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.shelf_ind_light = faces;
    }

    /// The library indices of one machine's carts, in library order — the view a system shelf
    /// shows. A card can carry all three machines at once, so this is how the row is narrowed
    /// to the one whose shelf is up.
    /// One machine's shelf, in the order it shows. Kept as a call rather than a field so the
    /// shelf stays the only thing that knows how a machine's list is built.
    fn system_view(&self, system: System) -> Vec<usize> {
        self.shelf.machine_view(system)
    }

    /// Put one machine's shelf up: rebuild the view to its carts, drop any favourites sub-view,
    /// park the caret at the front, and remember the choice on the card.
    fn show_system(&mut self, system: System) {
        self.shelf_system = system;
        // The row is laid out for whichever machine is up: an Advance cart is landscape and a
        // Game Boy cart portrait, so the geometry travels with the shelf rather than the app.
        self.shelf.set_system(system);
        // ...and so does the row's *frame*: the shadow under a dimmed cart and the blank cart a
        // not-yet-built face stands in as are both cut to one machine's shape, and they were cut
        // for the shelf this device came up on. Raised here and taken by the frontend, which is
        // the only party with a compositor.
        self.shelf_dirty = true;
        self.fav_view = false;
        let view = self.system_view(system);
        self.shelf.set_view(view, None);
        self.retally_letters();
        if let Some(root) = &self.root {
            crate::root::write_shelf_system(root, system);
        }
    }

    /// SELECT+L2: start a swap to the next machine that has any carts. Nothing happens — and
    /// the press is refused, so it reads as answered — on a card that carries only one machine.
    /// A swap already in flight swallows the press, so a second one cannot stack on the first.
    fn cycle_system(&mut self) {
        if self.shelf_switch.is_some() || !self.on_shelf() || self.core_picker.is_some() {
            return;
        }
        const ORDER: [System; 3] = [System::Gba, System::Gb, System::Gbc];
        let cur = ORDER.iter().position(|&s| s == self.shelf_system).unwrap_or(0);
        for k in 1..=ORDER.len() {
            let to = ORDER[(cur + k) % ORDER.len()];
            if self.shelf.carts.iter().any(|c| c.system() == to) {
                // A held arrow belongs to the shelf that is leaving; the swap owns the row now.
                self.shelf.release_hold();
                self.letters.release_hold();
                self.shelf_switch = Some(ShelfSwitch {
                    from: self.shelf_system,
                    to,
                    t: 0.0,
                    swapped: false,
                });
                return;
            }
        }
        self.refuse();
    }

    /// The swap's clock and the row's motion. Runs every frame; with no swap in flight it
    /// simply clears the motion, so a still row is laid out exactly as before this existed.
    fn update_shelf_switch(&mut self, dt: f32) {
        let Some(mut sw) = self.shelf_switch.take() else {
            self.shelf.set_motion(None);
            return;
        };
        sw.t += dt;
        // The view swaps at the bottom of the ascend: the old carts are off the top of the
        // panel by then, so the new shelf appears to fall into the space they left.
        if !sw.swapped && sw.t >= SWITCH_ASCEND_S {
            sw.swapped = true;
            self.show_system(sw.to);
        }
        if sw.t >= SWITCH_ASCEND_S + SWITCH_DESCEND_S {
            self.shelf.set_motion(None);
            return;
        }
        let p = if sw.t < SWITCH_ASCEND_S {
            sw.t / SWITCH_ASCEND_S
        } else {
            (sw.t - SWITCH_ASCEND_S) / SWITCH_DESCEND_S
        };
        self.shelf.set_motion(Some(slot_ui::Motion {
            p: p.clamp(0.0, 1.0),
            ascend: !sw.swapped,
        }));
        self.shelf_switch = Some(sw);
    }

    /// The logo of the machine whose shelf is up, in the bottom-left corner — the mirror of the
    /// favourites star in the bottom-right. Only ever one logo: it *is* the shelf. While a swap
    /// runs, the change is a cross-dissolve — the old machine's logo fades out as the new one
    /// fades in, both in place — so it dissolves rather than cuts. A reading, not a control:
    /// SELECT+L2 is the control.
    fn draw_shelf_indicator(&self, out: &mut Vec<Draw>) {
        // The top of the bottom band, which is the box the badge is centred in.
        let y = OUT_H as f32 - MOUTH_H;
        match &self.shelf_switch {
            Some(sw) => {
                let total = SWITCH_ASCEND_S + SWITCH_DESCEND_S;
                let p = (sw.t / total).clamp(0.0, 1.0);
                // Smoothstep, so the dissolve eases in and out rather than fading linearly.
                let e = p * p * (3.0 - 2.0 * p);
                self.draw_shelf_logo(sw.from, 1.0 - e, y, out);
                self.draw_shelf_logo(sw.to, e, y, out);
            }
            None => self.draw_shelf_logo(self.shelf_system, 1.0, y, out),
        }
    }

    /// One machine logo, bottom-left, at `alpha`. Both halves of a cross-dissolve come through
    /// here, so they are placed identically and only their opacity differs.
    fn draw_shelf_logo(&self, system: System, alpha: f32, y: f32, out: &mut Vec<Draw>) {
        if alpha <= 0.0 {
            return;
        }
        let idx = match system {
            System::Gb => 0,
            System::Gbc => 1,
            System::Gba => 2,
        };
        let faces = match slot_ui::palette::mode() {
            slot_ui::Mode::Light => &self.shelf_ind_light,
            slot_ui::Mode::Dark => &self.shelf_ind_faces,
        };
        let Some(&(tex, w, h)) = faces.get(idx) else {
            return;
        };
        if h == 0 {
            return;
        }
        // Centred in the band's height, and a fixed margin in from the panel's left edge. A
        // card's own badge is contained in the box rather than stretched, so one that arrives a
        // different shape still sits right; the built-in word is the same width it always was.
        let cy = y + MOUTH_H / 2.0;
        let k = (SHELF_IND_W / w as f32).min(SHELF_IND_H / h as f32);
        let (sw, sh) = (w as f32 * k, h as f32 * k);
        // Centred on the clock's own centre at the other end of the top band, so the two left-hand
        // marks stand in one column.
        let clock_cx = STATUS_READ_MARGIN + self.shelf_clock.w as f32 / 2.0;
        out.push(Draw::Tex {
            x: clock_cx - sw / 2.0,
            y: cy - sh / 2.0,
            w: sw,
            h: sh,
            tex,
            alpha,
        });
    }

    /// One face per slot in `letters::SLOTS` order, each with the size it was rasterised at,
    /// uploaded at boot. The sizes come with them because a face cropped to its ink is as wide
    /// as the letter is, and drawing it without them would set every letter to one width.
    pub fn set_letter_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.letter_faces = faces;
    }

    /// The metal between two letters, with the size it was rasterised at — a face cropped to
    /// its ink is as wide as it is and no wider, so the size has to travel with the id the same
    /// way a letter's does.
    pub fn set_letter_ridge_face(&mut self, ridge: (TexId, u32, u32)) {
        self.letter_ridge_face = Some(ridge);
    }

    /// Left or right on the shelf: move the index one letter and bring the row with it.
    ///
    /// The ring keeps the repeat, so this is the press and nothing else. The caret is seated on
    /// the first cart of the letter it landed on — that is the whole point of the index — and
    /// the row is handed the move as a glide, so it sweeps across every cart it is skipping
    /// rather than arriving as if only one had moved.
    ///
    /// `by` is the direction of the step: +1 for right (the next letter), -1 for left (the
    /// previous one).
    fn step_letters(&mut self, by: i32, now: Millis) {
        // The dial counts the library and the favourites shelf is narrower than it, so a
        // letter jump could seat the caret on a cart this view does not hold. Letter nav is
        // the library's own control and stands down while a view is filtered; the arrows
        // still walk the row.
        if self.fav_view {
            return;
        }
        let counts = self.letter_counts;
        if !self.letters.hold(by, now, &counts) {
            return;
        }
        self.seat_on_letter();
    }

    /// The same move, without a press behind it: the repeat of a held key.
    fn repeat_letters(&mut self, now: Millis) {
        if self.fav_view {
            return;
        }
        let counts = self.letter_counts;
        if self.letters.tick(now, &counts) {
            self.seat_on_letter();
        }
    }

    /// Puts the caret on the first cart of the letter the strip is showing, and hands the row
    /// the move as a glide so it sweeps across every cart it is skipping rather than flicking
    /// through only the last one. The direction is the row's own to take — it is a ring, and
    /// the glide runs to the nearest image of the chosen cart — so the strip and the row agree
    /// about which way is "onward" without this having to say so.
    fn seat_on_letter(&mut self) {
        if self.fav_view {
            return;
        }
        let slot = self.letters.target();
        // Found in the *view*, not the library: the shelf holds one machine's carts, and the
        // strip counts those (`retally_letters`), so `index` must be a view position.
        let Some(i) = self.shelf.view_position_of_letter(slot) else {
            // A slot the strip can only have reached by being counted as non-empty, so this is
            // unreachable; the next frame's `centre_on` puts it back if it ever happens.
            return;
        };
        self.shelf.index = i;
        // A held arrow key is a direction the shelf is no longer travelling in, and leaving
        // it armed would fire a cart step over the letter that was just chosen.
        self.shelf.release_hold();
        self.shelf.glide_to_target();
    }

    pub fn face_wants(&self) -> Vec<FaceWant> {
        let here = self.shelf();
        let screen = here.on_screen();
        let mut wants = screen.clone();
        for i in here.around(FACE_AHEAD) {
            if !wants.contains(&i) {
                wants.push(i);
            }
        }
        wants
            .into_iter()
            .filter(|i| here.face_of(*i).is_none())
            .map(|index| FaceWant {
                shelf: 0,
                index,
                on_screen: screen.contains(&index),
            })
            .collect()
    }

    pub fn face_cart(&self, _shelf: usize, index: usize) -> Option<&Cart> {
        self.shelf.carts.get(index)
    }

    pub fn set_cart_face(&mut self, shelf: usize, index: usize, tex: TexId) -> Option<TexId> {
        if shelf != 0 {
            return None;
        }
        let prev = self.shelf.face_of(index);
        self.shelf.set_face(index, Some(tex));
        prev
    }

    pub fn shed_faces(&mut self) -> Vec<TexId> {
        let keep = self.shelf.around(FACE_KEEP);
        let mut shed = Vec::new();
        for i in 0..self.shelf.carts.len() {
            if !keep.contains(&i) {
                shed.extend(self.shelf.take_face(i));
            }
        }
        shed
    }

    /// Handed over when the core is spawned, which is on the way into the slot.
    pub fn set_snapshot(&mut self, snapshot: Box<dyn Snapshot>) {
        self.snapshot = Some(snapshot);
    }

    pub fn set_core(&mut self, core: Core) {
        self.core = core;
    }

    pub fn set_platform(&mut self, platform: Platform) {
        self.platform = platform;
    }

    pub fn set_named_core(&mut self, named: bool) {
        self.named_core = named;
    }

    pub fn set_video_mode(&mut self, mode: VideoMode) {
        self.video_mode = mode;
    }

    pub fn source_rect(&self) -> [f32; 4] {
        video_mode::source_rect(self.platform, self.video_mode)
    }

    fn slot_owns_the_shoulders(&self) -> bool {
        matches!(self.phase, Phase::Playing { .. }) && self.platform != Platform::Gba
    }

    pub fn taken_buttons(&self) -> &'static [Btn] {
        if self.slot_owns_the_shoulders() {
            &[Btn::L1, Btn::R1]
        } else {
            &[]
        }
    }

    pub fn takes_from_the_game(&self, action: Action) -> bool {
        match action {
            Action::GbaDown(btn) | Action::GbaUp(btn) => self.taken_buttons().contains(&btn),
            _ => false,
        }
    }

    fn set_picture(&mut self, mode: VideoMode) {
        if self.video_mode == mode {
            return;
        }
        self.video_mode = mode;
        let (Some(root), Phase::Playing { cart }) = (self.root.clone(), &self.phase) else {
            return;
        };
        let written = video_mode::write_video_mode(&root, cart, mode);
        if let Err(e) = &written {
            eprintln!("slot: video: could not write video_mode.txt: {e}");
        }
        self.card_failing.store(written.is_err(), Ordering::Relaxed);
    }

    pub fn set_link_loaded(&mut self, serial: &'static str) {
        self.link_loaded = Some(serial);
    }

    pub fn link_mode(&self, stem: &str) -> (LinkKind, &'static str) {
        let Some((cart, auto)) = self.auto_link(stem) else {
            return (LinkKind::Cable, "auto");
        };
        let chosen = self.link_choices.get(stem).copied().unwrap_or(auto);
        (chosen, serial_option(chosen, auto, &cart.code, &cart.title))
    }

    pub fn link_active(&self) -> bool {
        self.link.is_some()
    }

    pub fn begin_link(&mut self, client_id: u16) {
        self.link = Some(LinkSession {
            client_id,
            lost_at: None,
        });
        self.sync_link_badge();
    }

    pub fn link_client_id(&self) -> Option<u16> {
        self.link.as_ref().map(|s| s.client_id)
    }

    pub fn may_rewind(&self) -> bool {
        self.state.rewind && !self.link_active()
    }

    pub fn turbo(&self) -> bool {
        self.state.turbo
    }

    pub fn records_rewind(&self) -> bool {
        self.state.rewind
    }

    pub fn picture_rect(&self) -> [f32; 4] {
        let (w, h) = self.platform.picture();
        let (sw, sh) = (slot_gfx::SRC_W as f32, slot_gfx::SRC_H as f32);
        let (x, y) = ((sw - w as f32) / 2.0, (sh - h as f32) / 2.0);
        [x / sw, y / sh, (x + w as f32) / sw, (y + h as f32) / sh]
    }

    pub fn may_load_state(&self) -> bool {
        !self.link_active()
    }

    pub fn may_fast_forward(&self) -> bool {
        !self.link_active()
    }

    pub fn end_link(&mut self) {
        self.link = None;
        self.link_player = None;
        self.sync_link_badge();
        self.radio.ask(RadioJob::Down);
        self.radio.ask(RadioJob::Cool);
    }

    pub fn peer_lost(&mut self) {
        if matches!(self.game_menu, Some(GameMenu::Linked { .. })) {
            self.game_menu = None;
        }
        let now = self.now();
        if let Some(session) = &mut self.link {
            session.lost_at.get_or_insert(now);
        }
        self.sync_link_badge();
    }

    pub fn bios_mismatch(&mut self) {
        if !self.link_active() {
            return;
        }
        let role = self
            .link_client_id()
            .map_or(self.last_role, LinkRow::from_client_id);
        self.end_link();
        self.hud.toast(Toast::BiosMismatch, self.now());
        self.unplug(role);
    }

    pub fn peer_ended(&mut self) {
        if !self.link_active() {
            return;
        }
        let role = self
            .link_client_id()
            .map_or(self.last_role, LinkRow::from_client_id);
        self.end_link();
        self.hud.toast(Toast::PeerEnded, self.now());
        self.unplug(role);
    }

    pub fn link_badge(&self) -> LinkBadge {
        self.hud.link()
    }

    pub fn set_link_badge_faces(&mut self, faces: Vec<TexId>) {
        self.hud.set_link_faces(faces);
    }

    fn sync_link_badge(&mut self) {
        let badge = match &self.link {
            None => LinkBadge::Off,
            Some(s) => match (s.client_id == 0, s.lost_at.is_some()) {
                (true, false) => LinkBadge::Hosting,
                (false, false) => LinkBadge::Joined,
                (true, true) => LinkBadge::HostingLost,
                (false, true) => LinkBadge::JoinedLost,
            },
        };
        self.hud.set_link(badge);
    }

    pub fn take_sfx(&mut self) -> Option<Sfx> {
        self.sfx.take()
    }

    pub fn set_power(&mut self, mut power: Power) {
        power.set_backlight(self.state.brightness);
        self.headphones = power.headphones();
        self.hud.set_headphones(self.headphones);
        let secs = power.now();
        if matches!(self.phase, Phase::SetClock { .. }) || secs < CLOCK_FLOOR {
            self.phase = clock_screen(secs, 0, false);
        }
        self.power = Some(power);
        self.battery_at = self.now();
        self.charge_at = self.now();
        self.last_led = None;
    }

    pub fn set_rumble(&mut self, strength: u16) {
        if let Some(power) = &mut self.power {
            power.set_rumble(strength);
        }
    }

    pub fn rumble_enabled(&self) -> bool {
        self.state.rumble
    }

    pub fn ff_speed(&self) -> u8 {
        self.state.ff_speed
    }

    pub fn ff_sound(&self) -> bool {
        self.state.ff_sound
    }

    pub fn screen_shader(&self) -> Shader {
        match self.platform {
            Platform::Gba => self.state.shader_gba,
            Platform::Gb | Platform::Gbc => self.state.shader_gb,
        }
    }

    pub fn colour_correction(&self) -> bool {
        self.state.colour_correction
    }

    pub fn powering_off(&self) -> bool {
        self.powering_off
    }

    pub fn ready_to_power_off(&self) -> bool {
        self.powering_off && self.now() >= self.act_at
    }

    pub fn ready_to_restart(&self) -> bool {
        self.restarting && self.now() >= self.act_at
    }

    pub fn shutting_down(&self) -> bool {
        self.powering_off || self.restarting
    }

    pub fn restarting(&self) -> bool {
        self.restarting
    }

    pub fn restart(&mut self) {
        self.settle_saves();
        if let Some(power) = &mut self.power {
            power.restart();
        }
    }

    pub fn power_menu(&self) -> Option<usize> {
        self.power_menu
    }

    pub fn core_picker(&self) -> Option<Core> {
        self.core_picker.map(|p| p.seat())
    }

    pub fn core_picker_chip(&self) -> Option<Chip> {
        self.core_picker.map(|p| p.chip(self.now()))
    }

    pub fn game_menu_open(&self) -> bool {
        self.game_menu.is_some()
    }

    pub fn game_menu(&self) -> Option<GameMenu> {
        self.game_menu
    }

    pub fn take_link_transport(&mut self) -> Option<(u16, Box<dyn LinkChannel>)> {
        self.link_transport.take()
    }

    pub fn take_link_reload(&mut self) -> Option<(String, &'static str)> {
        self.link_reload.take()
    }

    pub fn link_player(&self) -> Option<u8> {
        self.link_player
    }

    pub fn take_colour_correction(&mut self) -> Option<bool> {
        self.colour_pending.take()
    }

    pub fn gb_palette(&self) -> Option<GbPalette> {
        self.state.gb_palettes.then_some(self.state.gb_palette)
    }

    pub fn set_palette_live(&mut self, live: bool) {
        self.palette_live = live;
    }

    pub fn palette_live(&self) -> bool {
        self.palette_live
    }

    pub fn take_gb_palette(&mut self) -> Option<GbPalette> {
        self.palette_pending.take()
    }

    pub fn core(&self) -> Core {
        self.core
    }

    pub fn link_reload_done(&mut self) {
        let Some(reload) = self.reload.take() else {
            return;
        };
        if reload.fallback {
            self.link_choices.insert(reload.stem, reload.from);
            self.close_game_menu();
            return self.refuse();
        }
        if reload.cancelled {
            return self.close_game_menu();
        }
        let since = match self.game_menu {
            Some(GameMenu::Working { since, .. }) => since,
            _ => self.now(),
        };
        self.start_link_from(
            LinkStarter::spawn(reload.role.role(), link_port()),
            reload.role.client_id(),
            since,
        );
    }

    pub fn link_reload_failed(&mut self) {
        let Some(mut reload) = self.reload.take() else {
            return;
        };
        if !reload.fallback {
            self.link_reload = Some((reload.stem.clone(), reload.from_serial));
            reload.fallback = true;
            self.reload = Some(reload);
            return;
        }
        self.link_choices.insert(reload.stem, reload.from);
        self.refuse_seated();
    }

    pub fn selected_stem(&self) -> Option<&str> {
        self.shelf.current_cart().map(|c| c.stem.as_str())
    }

    /// Which machine's picture belongs on the panel.
    ///
    /// The card's three shelves collapse into the renderer's two here, because Game Boy and
    /// Game Boy Color hand back the same 160x144 frame — `slot_store::System` is the three and
    /// `slot_gfx::System` is the two. `Gba` stands in for an empty shelf, where there is no
    /// picture to misplace either way.
    pub fn screen_system(&self) -> ScreenSystem {
        match self.shelf.current_cart().map(|c| c.system()) {
            Some(System::Gb) | Some(System::Gbc) => ScreenSystem::Gb,
            _ => ScreenSystem::Gba,
        }
    }

    pub fn battery(&self) -> Option<Battery> {
        self.battery
    }

    pub fn poweroff(&mut self) {
        self.settle_saves();
        if let Some(power) = &mut self.power {
            power.poweroff();
        }
    }

    pub fn apply(&mut self, action: Action) {
        // A system-shelf swap owns the screen for its two beats: every key is inert until it
        // lands, so nothing can be pressed into the middle of the movement. `Session::act`
        // gates the volume and mute on `input_locked` for the same reason — those are handled
        // on the session's side of `apply` and would otherwise slip through.
        if self.shelf_switch.is_some() {
            return;
        }
        // Ahead of everything, including the device's own keys: the menu is a decision the
        // user is in the middle of making, and a volume press underneath it would be one
        // more thing happening while they read.
        if self.power_menu.is_some() {
            match action {
                Action::LidClose => return self.doze(),
                Action::LidOpen => return self.wake(),
                _ => return self.power_menu_input(action),
            }
        }
        // (WiFi 直连对话框的输入拦截已移除——功能暂缓。)
        // The cheat table owns every button on the game's side while it is up: up/down move, A
        // toggles the row, B leaves, and the lid is honoured so the device can still sleep under it.
        if self.cheat_menu.is_some() {
            match action {
                Action::LidClose => return self.doze(),
                Action::LidOpen => return self.wake(),
                _ => return self.cheat_menu_input(action),
            }
        }
        // The palette browser owns them the same way while it is up: it is a modal over a paused
        // game, and the lid is honoured so the device can still sleep under it.
        if self.pal_cursor.is_some() {
            match action {
                Action::LidClose => return self.doze(),
                Action::LidOpen => return self.wake(),
                _ => return self.palette_browser_input(action),
            }
        }
        // The lid, the light and the sound belong to the device rather than to whatever is
        // on screen, so they are taken before the phase gets a look at the action.
        match action {
            Action::LidClose => return self.doze(),
            Action::LidOpen => return self.wake(),
            Action::PowerPress => {
                if self.link_active() {
                    self.end_link();
                    return;
                }
                self.woke_on_press = matches!(self.phase, Phase::Doze { .. });
                if self.woke_on_press {
                    self.wake();
                }
                return self.flush_resume();
            }
            Action::PowerTap => return self.power_press(),
            Action::PowerHold => return self.open_power_menu(),
            Action::PowerOff => return,
            _ => {}
        }
        if let Phase::SetClock {
            picker, from_menu, ..
        } = &mut self.phase
        {
            match action {
                Action::GbaDown(Btn::Left) | Action::ShelfLeft => picker.left(),
                Action::GbaDown(Btn::Right) | Action::ShelfRight => picker.right(),
                Action::GbaDown(Btn::Up) => picker.up(),
                Action::GbaDown(Btn::Down) => picker.down(),
                Action::GbaDown(Btn::A) | Action::Insert => self.confirm_clock(),
                Action::GbaDown(Btn::B) if *from_menu => {
                    self.phase = Phase::QuickMenu {
                        row: QuickRow::DateTime,
                    }
                }
                _ => {}
            }
            return;
        }
        if self.adjust(action) {
            return;
        }
        match action {
            Action::GbaUp(Btn::Left) => self.shelf_mut().release_left(),
            Action::GbaUp(Btn::Right) => self.shelf_mut().release_right(),
            _ => {}
        }
        if self.game_menu.is_some() {
            return self.game_menu_input(action);
        }
        let now = self.now();
        // Ahead of the match that borrows the phase: the card's limit is a fact about how many
        // rows are on it, not about the press that is being answered.
        let about_max = self.about_scroll_max();
        match self.phase {
            Phase::Shelf => match action {
                Action::GbaDown(Btn::Start) if self.core_picker.is_none() => {
                    self.open_core_picker()
                }
                Action::AudioProfileNext if self.core_picker.is_none() => self.cycle_audio(true),
                Action::AudioProfilePrev if self.core_picker.is_none() => self.cycle_audio(false),
                Action::ModeToggle if self.core_picker.is_none() => self.toggle_mode(),
                _ if self.core_picker.is_some() => self.core_picker_input(action),
                Action::GbaDown(Btn::Up) => self.jump_letter(-1),
                Action::GbaDown(Btn::Down) => self.jump_letter(1),
                Action::ShelfLeft | Action::GbaDown(Btn::Left) => self.shelf.hold_left(now),
                Action::ShelfRight | Action::GbaDown(Btn::Right) => self.shelf.hold_right(now),
                Action::GbaDown(Btn::L1) => self.step_letters(-1, now),
                Action::GbaDown(Btn::R1) => self.step_letters(1, now),
                Action::GbaUp(Btn::L1) => self.letters.release(-1),
                Action::GbaUp(Btn::R1) => self.letters.release(1),
                Action::OpenAbout => {
                    self.phase = Phase::About {
                        scroll: 0.0,
                        want: 0.0,
                        from_menu: false,
                    }
                }
                Action::QuickMenu => self.open_quick_menu(),
                Action::GbaDown(Btn::A) => self.play_held = Some(now),
                Action::GbaUp(Btn::A) => {
                    if self.play_held.take().is_some() {
                        self.insert(!self.state.eject_save);
                    }
                }
                Action::GbaDown(Btn::X) => self.fav_held = Some(now),
                Action::GbaUp(Btn::X) => {
                    if self.fav_held.take().is_some() {
                        self.toggle_favorite();
                    }
                }
                Action::GbaDown(Btn::Y) => self.sys_held = Some(now),
                Action::GbaUp(Btn::Y) => {
                    if self.sys_held.take().is_some() {
                        self.cycle_folder();
                    }
                }
                Action::Insert => self.insert(!self.state.eject_save),
                _ => {}
            },
            Phase::Inserting { .. } if action == Action::Eject => self.eject(),
            Phase::Playing { .. } => match action {
                Action::GbaDown(Btn::L1) if self.slot_owns_the_shoulders() => {
                    self.set_picture(VideoMode::Stretch)
                }
                Action::GbaDown(Btn::R1) if self.slot_owns_the_shoulders() => {
                    self.set_picture(VideoMode::Actual)
                }
                Action::Eject => self.eject(),
                Action::MaskCycle if self.current_machine() != System::Gba => self.overlay_next(),
                Action::MaskCycle => self.cycle_mask(),
                Action::ColorCycle => self.cycle_cc(),
                Action::ColorHold => self.open_palette_browser(),
                Action::GameMenu => self.game_menu_shortcut(),
                Action::Polaroids => self.open_polaroids(),
                Action::SaveState => self.save_state(),
                Action::LoadState => self.load_newest(),
                Action::RewindStart if !self.may_rewind() => self.refuse(),
                Action::FfStart if !self.may_fast_forward() => self.refuse(),
                _ => {}
            },
            Phase::Polaroids { .. } => match action {
                Action::ShelfLeft | Action::GbaDown(Btn::Left) => self.flick(Polaroids::left),
                Action::ShelfRight | Action::GbaDown(Btn::Right) => self.flick(Polaroids::right),
                Action::GbaDown(Btn::A) => self.load_selected(),
                Action::GbaDown(Btn::B) | Action::Polaroids => self.close_polaroids(),
                Action::GbaDown(Btn::X) => self.undo(self.now()),
                Action::GbaDown(Btn::Y) => self.delete_selected(),
                _ => {}
            },
            Phase::About {
                scroll,
                want,
                from_menu,
            } => match action {
                Action::GbaDown(Btn::Up) => {
                    let w = (want - ABOUT_PAGE).max(0.0);
                    self.phase = Phase::About {
                        scroll,
                        want: w,
                        from_menu,
                    };
                }
                Action::GbaDown(Btn::Down) => {
                    let w = (want + ABOUT_PAGE).min(about_max);
                    self.phase = Phase::About {
                        scroll,
                        want: w,
                        from_menu,
                    };
                }
                _ if action == Action::GbaDown(Btn::B)
                    || action == Action::OpenAbout
                    || action == Action::QuickMenu =>
                {
                    self.phase = if from_menu || action == Action::QuickMenu {
                        Phase::QuickMenu {
                            row: QuickRow::About,
                        }
                    } else {
                        Phase::Shelf
                    };
                }
                _ => {}
            },
            Phase::QuickMenu { row } => self.quick_menu_input(row, action),
            _ => {}
        }
    }

    fn open_quick_menu(&mut self) {
        self.phase = Phase::QuickMenu {
            row: QuickRow::MAIN[0],
        };
    }

    fn quick_menu_input(&mut self, row: QuickRow, action: Action) {
        let row = match action {
            Action::GbaDown(Btn::Up) => row.up(),
            Action::GbaDown(Btn::Down) => row.down(),
            Action::GbaDown(Btn::Left) => return self.change_setting(row, false),
            Action::GbaDown(Btn::Right) => return self.change_setting(row, true),
            Action::GbaDown(Btn::A) => return self.open_quick_row(row),
            Action::GbaDown(Btn::B) | Action::QuickMenu => {
                self.phase = match row.parent() {
                    Some(row) => Phase::QuickMenu { row },
                    None => Phase::Shelf,
                };
                return;
            }
            _ => return,
        };
        self.phase = Phase::QuickMenu { row };
    }

    fn open_quick_row(&mut self, row: QuickRow) {
        match row {
            QuickRow::DateTime => {
                self.phase = clock_screen(self.utc_secs(), self.state.utc_offset_min, true);
            }
            QuickRow::About => {
                self.phase = Phase::About {
                    scroll: 0.0,
                    want: 0.0,
                    from_menu: true,
                }
            }
            QuickRow::Screen | QuickRow::Game => {
                if let Some(row) = row.child() {
                    self.phase = Phase::QuickMenu { row };
                }
            }
            QuickRow::EjectSave
            | QuickRow::Turbo
            | QuickRow::Rewind
            | QuickRow::FastForward
            | QuickRow::FastForwardSound
            | QuickRow::ColourCorrection
            | QuickRow::GbaShader
            | QuickRow::GbShader
            | QuickRow::GbPalettes
            | QuickRow::Rumble => {}
        }
    }

    fn change_setting(&mut self, row: QuickRow, right: bool) {
        let s = &mut self.state;
        match row {
            QuickRow::FastForward => {
                let to = ff_next(s.ff_speed, right);
                if to == s.ff_speed {
                    return;
                }
                s.ff_speed = to;
            }
            QuickRow::FastForwardSound => s.ff_sound = !s.ff_sound,
            QuickRow::ColourCorrection => {
                s.colour_correction = !s.colour_correction;
                self.colour_pending = Some(s.colour_correction);
            }
            QuickRow::GbaShader => {
                let to = s.shader_gba.step(&Shader::GBA, right);
                if to == s.shader_gba {
                    return;
                }
                s.shader_gba = to;
            }
            QuickRow::GbShader => {
                let to = s.shader_gb.step(&Shader::GB, right);
                if to == s.shader_gb {
                    return;
                }
                s.shader_gb = to;
            }
            QuickRow::Rumble => s.rumble = !s.rumble,
            QuickRow::EjectSave => s.eject_save = !s.eject_save,
            QuickRow::Turbo => s.turbo = !s.turbo,
            QuickRow::Rewind => s.rewind = !s.rewind,
            QuickRow::GbPalettes => s.gb_palettes = !s.gb_palettes,
            QuickRow::DateTime | QuickRow::About | QuickRow::Screen | QuickRow::Game => return,
        }
        self.persist();
    }

    pub fn apply_at(&mut self, action: Action, now: Millis) {
        self.clock = now as f64;
        self.apply(action);
    }

    fn step_shader(&mut self, right: bool) {
        if !matches!(self.phase, Phase::Inserting { .. } | Phase::Playing { .. }) {
            return;
        }
        let (shader, choices) = match self.platform {
            Platform::Gba => (&mut self.state.shader_gba, &Shader::GBA[..]),
            Platform::Gb | Platform::Gbc => (&mut self.state.shader_gb, &Shader::GB[..]),
        };
        *shader = shader.step(choices, right);
        let said = match *shader {
            Shader::Off => Toast::ShaderOff,
            Shader::Lcd3x => Toast::ShaderLcd3x,
            Shader::Grid => Toast::ShaderGrid,
            Shader::Dot => Toast::ShaderDot,
            Shader::Simpletex => Toast::ShaderSimpletex,
        };
        self.persist();
        self.hud.toast(said, self.now());
    }

    fn adjust(&mut self, action: Action) -> bool {
        if action == Action::MuteToggle {
            self.mute_toggle();
            return true;
        }
        if let Action::ShaderNext | Action::ShaderPrev = action {
            self.step_shader(action == Action::ShaderNext);
            return true;
        }
        if let Action::PaletteNext | Action::PalettePrev = action {
            if self.palette_live && matches!(self.phase, Phase::Playing { .. }) {
                let to = match action {
                    Action::PalettePrev => self.state.gb_palette.prev(),
                    _ => self.state.gb_palette.next(),
                };
                self.state.gb_palette = to;
                self.palette_pending = Some(to);
                self.persist();
                self.hud.toast(Toast::Palette(to), self.now());
            }
            return true;
        }
        if action == Action::ColourCorrectionToggle {
            self.change_setting(QuickRow::ColourCorrection, true);
            let said = match self.state.colour_correction {
                true => Toast::ColourOn,
                false => Toast::ColourOff,
            };
            self.hud.toast(said, self.now());
            return true;
        }
        let s = &self.state;
        let (kind, value) = match action {
            Action::BrightnessUp => (HudKind::Brightness, up(s.brightness, 1, BRIGHTNESS_MAX)),
            Action::BrightnessDown => (HudKind::Brightness, s.brightness.saturating_sub(1)),
            Action::BlueLightUp => (HudKind::BlueLight, up(s.blue_light, 1, BLUE_LIGHT_MAX)),
            Action::BlueLightDown => (HudKind::BlueLight, s.blue_light.saturating_sub(1)),
            Action::VolumeUp => (HudKind::Volume, up(self.level(), VOLUME_STEP, VOLUME_MAX)),
            Action::VolumeDown => (HudKind::Volume, self.level().saturating_sub(VOLUME_STEP)),
            _ => return false,
        };
        if kind == HudKind::Volume {
            self.remember_volume();
        }
        let level = match kind {
            HudKind::Brightness => &mut self.state.brightness,
            HudKind::BlueLight => &mut self.state.blue_light,
            HudKind::Volume if self.headphones => &mut self.state.volume_hp,
            HudKind::Volume => &mut self.state.volume,
            HudKind::Rewind => return false,
        };
        let moved = *level != value;
        *level = value;
        let unmuted = kind == HudKind::Volume && std::mem::take(self.muted_mut());
        let (shown, now) = (self.hud_value(kind, value), self.now());
        self.hud.show(kind, shown, self.muted(), now);
        if let (HudKind::Brightness, Some(power)) = (kind, &mut self.power) {
            power.set_backlight(value);
        }
        if moved || unmuted {
            self.persist_soon();
        }
        true
    }

    fn remember_volume(&mut self) {
        if self.vol_before.len() == 2 {
            self.vol_before.remove(0);
        }
        self.vol_before
            .push((self.level(), self.muted(), self.now()));
    }

    fn mute_toggle(&mut self) {
        let now = self.now();
        if let Some((volume, muted, _)) = self
            .vol_before
            .iter()
            .find(|(_, _, at)| now.saturating_sub(*at) <= MUTE_CHORD_MS)
            .copied()
        {
            *self.level_mut() = volume;
            *self.muted_mut() = muted;
        }
        self.vol_before.clear();
        let muted = !self.muted();
        *self.muted_mut() = muted;
        self.hud
            .show(HudKind::Volume, self.output_volume(), muted, now);
        self.persist_soon();
    }

    fn hud_value(&self, kind: HudKind, value: u8) -> u8 {
        match kind {
            HudKind::Volume => self.output_volume(),
            _ => value,
        }
    }

    pub fn show_rewind(&mut self, fill: u8) {
        let now = self.now();
        self.hud.show(HudKind::Rewind, fill, false, now);
    }

    pub fn hide_rewind(&mut self) {
        self.hud.release_rewind();
    }

    pub fn set_ff(&mut self, ff: FfState) {
        self.hud.set_ff(ff);
    }

    pub fn ff_badge(&self) -> Option<Icon> {
        self.hud.badge()
    }

    pub fn blue_light(&self) -> u8 {
        self.state.blue_light
    }

    pub fn brightness(&self) -> u8 {
        self.state.brightness
    }

    pub fn volume(&self) -> u8 {
        self.level()
    }

    fn level(&self) -> u8 {
        if self.headphones {
            self.state.volume_hp
        } else {
            self.state.volume
        }
    }

    fn level_mut(&mut self) -> &mut u8 {
        if self.headphones {
            &mut self.state.volume_hp
        } else {
            &mut self.state.volume
        }
    }

    pub fn muted(&self) -> bool {
        if self.headphones {
            self.state.muted_hp
        } else {
            self.state.muted
        }
    }

    fn muted_mut(&mut self) -> &mut bool {
        if self.headphones {
            &mut self.state.muted_hp
        } else {
            &mut self.state.muted
        }
    }

    pub fn output_volume(&self) -> u8 {
        if self.muted() {
            0
        } else {
            self.level()
        }
    }

    pub fn hud_icon(&self) -> Icon {
        self.hud.glyph()
    }

    pub fn now(&self) -> Millis {
        self.clock as Millis
    }

    fn flick(&mut self, step: fn(&mut Polaroids)) {
        if let Some(p) = &mut self.polaroids {
            step(p);
        }
    }

    pub fn update(&mut self, dt: f32) {
        self.clock += dt as f64 * 1000.0;
        if self.name_pending && self.shelf_platform.face.is_some() {
            self.name_pending = false;
            self.shelf_named = Some(self.now());
        }
        self.timers();
        // The system-shelf swap, before the phase walk: it advances the beats, swaps the view
        // at the bottom of the ascend, and hands the row its motion for this frame. With no
        // swap in flight it clears the motion, so a still row is laid out as it always was.
        self.update_shelf_switch(dt);
        // A queue poll rather than a syscall, so the frame loop can afford it every frame —
        // which is the whole reason the slow parts of starting a link are on a thread of
        // their own.
        self.poll_link();
        let now = self.now();
        let ready = self.core_faces_ready();
        if let Some(picker) = &mut self.core_picker {
            if picker.waiting() && (ready || picker.waited(now) >= FACES_WAIT_MS) {
                picker.start(now);
            }
        }
        if self.core_picker.is_some_and(|p| p.finished(now)) {
            self.core_picker = None;
        }
        if !self.on_shelf() {
            self.shelf.release_hold();
            self.letters.release_hold();
        }
        let mut touched = false;
        let next = match &mut self.phase {
            Phase::Shelf => {
                self.shelf.tick(now);
                self.shelf.update(dt);
                None
            }
            Phase::Inserting {
                cart,
                t,
                core_ready,
                resumed,
                ..
            } => {
                let was = *t;
                *t += dt;
                let at = SEATED_AT - Sfx::Insert.lead();
                touched = !*resumed && was < at && *t >= at;
                (*t >= INSERT_S && *core_ready).then(|| Phase::Playing {
                    cart: std::mem::take(cart),
                })
            }
            Phase::Ejecting { t, .. } => {
                if self.screen <= 0.0 {
                    let was = *t;
                    *t += dt;
                    touched = was < 0.0 && *t >= 0.0;
                }
                (*t >= EJECT_S).then_some(Phase::Shelf)
            }
            // The card closes on where the last press asked it to go rather than arriving there
            // between two frames: a page of text that changed while the eye was mid-line is a
            // page that has to be found again. `want` is the press and `scroll` is the picture,
            // which is the same split the letter strip keeps.
            Phase::About { scroll, want, .. } => {
                if (*want - *scroll).abs() < 0.5 {
                    *scroll = *want;
                } else {
                    *scroll += (*want - *scroll) * (ABOUT_EASE * dt).min(1.0);
                }
                None
            }
            _ => None,
        };
        // The strip, after the row has moved rather than as part of it. Out here because it
        // needs the whole of `self` — the repeat reaches the caret, which is the shelf's — and
        // the match above is holding `phase` borrowed. The order within it is the order the
        // three things depend on each other: the repeat may move the marker, the spring may
        // move it further, and the last line puts the marker back on the cart the row is
        // actually showing. That last line is what makes the strip a readout as well as a
        // control — an arrow on the row moves the caret and the strip follows it with nothing
        // telling it to.
        if self.on_shelf() {
            self.repeat_letters(now);
            if let Some(cart) = self.shelf.current_cart() {
                self.letters.centre_on(cart.initial);
            }
            self.letters.settle(dt);
        }
        // One clip for the whole movement, and the only thing done to it is when it starts.
        if touched {
            self.sfx = Some(match self.phase {
                Phase::Ejecting { .. } => Sfx::Eject,
                _ => Sfx::Insert,
            });
        }
        if let Some(phase) = next {
            self.phase = phase;
            self.refused_from = None;
            let seated = match &self.phase {
                Phase::Playing { cart } => Some(cart.clone()),
                _ => None,
            };
            if seated.is_none() {
                self.link_loaded = None;
            }
            self.record_cart(seated);
        }
        self.step_screen(dt);
    }

    fn step_screen(&mut self, dt: f32) {
        let lit = matches!(self.phase, Phase::Playing { .. } | Phase::Polaroids { .. });
        let step = if lit {
            dt / POWER_ON_S
        } else {
            -dt / POWER_OFF_S
        };
        self.screen = (self.screen + step).clamp(0.0, 1.0);
        let docked = self.power_menu.is_none()
            && matches!(
                self.phase,
                Phase::Shelf | Phase::Inserting { .. } | Phase::Ejecting { .. }
            );
        let sliding = self.power_menu.is_none()
            && matches!(
                self.phase,
                Phase::Inserting { .. }
                    | Phase::Playing { .. }
                    | Phase::Polaroids { .. }
                    | Phase::Ejecting { .. }
            );
        let reach = if sliding { dt / POWER_ON_S } else { 1.0 };
        self.lift = match docked {
            true => (self.lift - reach).max(0.0),
            false => (self.lift + reach).min(1.0),
        };
    }

    pub fn frame_split(&self) -> Option<f32> {
        match (&self.phase, self.power_menu) {
            (Phase::QuickMenu { .. }, None) => Some(slot_ui::QUICK_SPLIT),
            _ => None,
        }
    }

    pub fn frame_lift(&self) -> f32 {
        self.lift
    }

    pub fn screen_power(&self) -> f32 {
        self.screen
    }

    pub fn set_game_ready(&mut self, ready: bool) {
        self.game_ready = ready;
    }

    pub fn game_ready(&self) -> bool {
        self.game_ready
    }

    pub fn game_visible(&self) -> bool {
        self.game_ready && self.screen > 0.0
    }

    pub fn tick_ms(&mut self, now: Millis) {
        self.clock = self.clock.max(now as f64);
        self.timers();
    }

    fn timers(&mut self) {
        self.play_hold();
        self.fav_hold();
        self.sys_hold();
        // The "connect the cable" beat hands over to the live screen on its own, a moment after
        // the gadget has been switched.
        // While the connect beat is up, poll for the address the enter script leaves behind: the
        // beat hands over to the live screen the moment one appears, however long the join took.
        if self.direct_link == Some(1) && self.now().saturating_sub(self.direct_at) >= 1000 {
            self.direct_at = self.now();
            if let Ok(ip) = std::fs::read_to_string("/tmp/slot-ip") {
                let ip = ip.trim().to_string();
                if !ip.is_empty() {
                    self.direct_ip = Some(ip);
                    self.direct_ip_dirty = true;
                    self.direct_link = Some(2);
                }
            }
        }
        // The grace period can run out with the switcher open, so the hint answers to the
        // clock rather than to whatever was on offer on the way in.
        let offer = self.undo_label();
        if let Some(p) = &mut self.polaroids {
            p.set_undo(offer);
        }
        if self.doze_expired() {
            self.on_doze_timeout();
        }
        if self.now() >= self.autosave_at {
            self.autosave();
        }
        if self.now() >= self.card_check_at {
            self.card_check_at = self.now() + CARD_CHECK_MS;
            self.card_read_only = self.power.as_ref().is_some_and(|p| p.card_read_only());
        }
        let failing = self.card_failing.load(Ordering::Relaxed) || self.card_read_only;
        if failing != self.hud.alarmed() {
            eprintln!(
                "slot: card: alarm {} (read-only {})",
                if failing { "on" } else { "off" },
                self.card_read_only
            );
        }
        self.hud.set_alarm(failing, self.now());
        if self.now() >= self.battery_at {
            self.battery_at = self.now() + BATTERY_POLL_MS;
            self.battery = self.power.as_ref().and_then(|p| p.battery());
            if let Some(b) = self.battery {
                self.on_battery(b);
            }
        }
        if self.now() >= self.headphones_at {
            self.headphones_at = self.now() + HEADPHONES_POLL_MS;
            let on = self.power.as_ref().is_some_and(|p| p.headphones());
            if on != self.headphones {
                self.headphones = on;
                self.hud.set_headphones(on);
                let now = self.now();
                self.hud
                    .show(HudKind::Volume, self.output_volume(), self.muted(), now);
            }
        }
        if self.now() >= self.charge_at {
            self.charge_at = self.now() + CHARGE_POLL_MS;
            if let (Some(power), Some(b)) = (self.power.as_ref(), self.battery.as_mut()) {
                b.charge = power.charge();
            }
            let state = self.led_state();
            self.set_led(state);
        }
        if let Some(GameMenu::Linked {
            since,
            opened: false,
            ..
        }) = self.game_menu
        {
            if self.now().saturating_sub(since) >= LINKED_HOLD_MS {
                self.game_menu = None;
            }
        }
        if let Some(GameMenu::Unplug { since, .. }) = self.game_menu {
            if self.now().saturating_sub(since) >= UNPLUG_HOLD_MS {
                self.game_menu = None;
            }
        }
        if let Some(at) = self.link.as_ref().and_then(|s| s.lost_at) {
            if self.now().saturating_sub(at) >= LINK_LOST_MS {
                self.end_link();
            }
        }
    }

    pub fn led_state(&self) -> LedState {
        let Some(b) = self.battery else {
            return LedState::Running;
        };
        match b.charge {
            Charge::Charging => LedState::Charging,
            Charge::Full => LedState::Charged,
            _ if b.percent <= BATTERY_LOW => LedState::Low,
            _ => LedState::Running,
        }
    }

    fn set_led(&mut self, state: LedState) {
        if self.shutting_down() && state != LedState::Off {
            return;
        }
        if self.last_led == Some(state) {
            return;
        }
        self.last_led = Some(state);
        if let Some(power) = self.power.as_mut() {
            power.set_led(state);
        }
    }

    fn record_cart(&mut self, cart: Option<String>) {
        let platform = self.seated_cart().map(|c| c.platform);
        if self.state.cart == cart && self.state.cart_platform == platform {
            return;
        }
        self.state.cart = cart;
        self.state.cart_platform = platform;
        self.persist();
    }

    fn persist(&self) {
        let Some(root) = &self.root else {
            return;
        };
        self.state_writer.flush();
        let written = write_slot_state(root, &self.state);
        if let Err(e) = &written {
            eprintln!("slot: slot.state: {e}");
        }
        self.card_failing.store(written.is_err(), Ordering::Relaxed);
    }

    fn persist_soon(&self) {
        if let Some(root) = &self.root {
            self.state_writer.save(root, &self.state);
        }
    }

    pub fn flush_state(&self) {
        self.state_writer.flush();
    }

    pub fn on_core_ready(&mut self) {
        let Phase::Inserting {
            cart, core_ready, ..
        } = &mut self.phase
        else {
            return;
        };
        *core_ready = true;
        let cart = cart.clone();
        self.retire_refused_resume(&cart);
    }

    fn retire_refused_resume(&mut self, stem: &str) {
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if snapshot.resume_trusted() {
            return;
        }
        if !self.named_core {
            return;
        }
        let Some(root) = &self.root else {
            return;
        };
        if let Some(h) = self.pending_save.take() {
            let _ = h.join();
        }
        if self.system_of(stem).is_none() {
            return;
        }
        let ring = StateRing::new(root, self.platform, self.core, stem);
        match ring.retire_resume(&format_stamp(self.wall_secs())) {
            Ok(Some(to)) => eprintln!(
                "slot: resume: {} refused this state, moved it to {}",
                self.core.as_str(),
                to.display()
            ),
            Ok(None) => {}
            Err(e) => eprintln!("slot: resume: could not move the refused state aside: {e}"),
        }
    }

    pub fn on_core_failed(&mut self) {
        let caught = self.seat();
        let Phase::Inserting { cart, .. } = &mut self.phase else {
            return;
        };
        let cart = std::mem::take(cart);
        self.refuse_out(cart, caught);
    }

    fn refuse_out(&mut self, cart: String, caught: f32) {
        let t = (1.0 - caught) * EJECT_S;
        self.phase = Phase::Ejecting { cart, t };
        self.refused_from = Some(t);
    }

    pub fn refuse(&mut self) {
        self.refusal = Some(Refusal::started(self.now()));
    }

    pub fn refusal_active(&self, now: Millis) -> bool {
        self.refusal.is_some_and(|r| r.active(now))
    }

    pub fn seat(&self) -> f32 {
        match &self.phase {
            Phase::Shelf => 0.0,
            Phase::Inserting { t, resumed, .. } => {
                if *resumed {
                    1.0
                } else {
                    (t / SEATED_AT).clamp(0.0, 1.0)
                }
            }
            Phase::Ejecting { t, .. } => 1.0 - (t / EJECT_S).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    pub fn draw(&self, out: &mut Vec<Draw>) {
        if let Some(index) = self.power_menu {
            self.draw_power_menu(index, out);
            return;
        }
        // (WiFi 直连对话框的绘制已移除——功能暂缓。)
        if let Some(index) = self.cheat_menu {
            self.draw_cheat_menu(index, out);
            return;
        }
        if self.shutting_down() {
            out.push(Draw::Rect {
                x: 0.0,
                y: 0.0,
                w: OUT_W as f32,
                h: OUT_H as f32,
                colour: [0.0, 0.0, 0.0, 1.0],
            });
            let which = if self.restarting {
                PowerChoice::Restart
            } else {
                PowerChoice::PowerOff
            };
            if let Some((tex, w, h)) = self.shutdown_faces.get(which.index()).copied() {
                out.push(Draw::Tex {
                    x: ((OUT_W - w) / 2) as f32,
                    y: ((OUT_H - h) / 2) as f32,
                    w: w as f32,
                    h: h as f32,
                    tex,
                    alpha: 1.0,
                });
            }
            return;
        }
        // Whether the letter band went down this frame. The HUD reads it to place its plate: the
        // band is only on the shelf, and it is the one phase where the top of the screen is not
        // free for a plate to sit against.
        let mut band_on = false;
        match &self.phase {
            Phase::SetClock {
                picker, from_menu, ..
            } => {
                let (line, hint) = match self.clock_faces {
                    Some((line, hint)) => (Some(line), Some(hint)),
                    None => (None, None),
                };
                let back = self
                    .quick_menu_faces
                    .as_ref()
                    .filter(|_| *from_menu)
                    .map(|f| f.legend[0]);
                picker.draw(line, hint, back, out);
                return;
            }
            Phase::QuickMenu { row } => QuickMenu {
                row: *row,
                values: QuickRow::ALL.map(|r| self.quick_value(r)),
                clock: self.quick_clock_faces,
                faces: self.quick_menu_faces.as_ref(),
            }
            .draw(out),
            Phase::Shelf => {
                draw_backdrop(self.wallpaper, out);
                match (self.core_picker_shown(), self.selected_stem()) {
                    (Some(picker), Some(stem)) => {
                        let open = ease(picker.openness(self.now()));
                        let dim = 1.0 + (CORE_PICKER_DIM - 1.0) * open;
                        self.shelf()
                            .draw_row(Some(stem), 0.0, CORE_PICKER_RECEDE * open, dim, out);
                        draw_empty_slot(out);
                    }
                    _ => {
                        self.shelf.draw(self.shelf_shake(), out);
                        // The stars, over the row and under the case's own furniture: a starred
                        // cart wears one above it, and the shelf's own star sits in the corner
                        // saying which shelf is up.
                        self.draw_fav_stars(out);
                        // The band at the top of the case, and the index printed along it. The
                        // band is the machine's — the cart bay's mirror, drawn from the bay's
                        // own numbers — so it goes down as furniture and the letters go over
                        // it. Not drawn while the picker has the lid off: the index says where
                        // in the library the caret is, and with a cart open there is no
                        // library on screen for it to be somewhere in.
                        draw_top_band(out);
                        // The light off both bands of the case, drawn with the band because the
                        // two boundaries are the same object seen at the top and the bottom of
                        // the screen. After the row, so a cart on its way into the slot is lit
                        // by it rather than drawn over it.
                        draw_edge_glow(out);
                        band_on = true;
                        // The letter strip is the library's index, and the favourites shelf is
                        // not the library: the dial has stood down, so the band is left with an
                        // empty window rather than an index that no longer means anything.
                        if !self.fav_view {
                            self.letters.draw_strip(
                                &self.letter_faces,
                                self.letter_ridge_face,
                                &self.letter_counts,
                                out,
                            );
                        }
                        self.draw_fav_indicator(out);
                        // Which machine's shelf is up, in the opposite bottom corner — the
                        // mirror of the favourites star.
                        self.draw_shelf_indicator(out);
                        draw_slot_name(self.shelf_platform, self.slot_name_alpha(), out);
                    }
                }
                // After the row and before the case: it names what the row is showing, so it
                // belongs to the gap the row leaves rather than to the plastic below it.
                if let Some((tex, w, _)) = self.shelf_title_face {
                    out.push(Draw::Tex {
                        x: (OUT_W as f32 - w as f32) / 2.0,
                        y: self.shelf_title_y(),
                        w: w as f32,
                        h: SHELF_TITLE_H as f32,
                        tex,
                        alpha: 1.0,
                    });
                }
                // Last of the shelf's own furniture, because it is printed on the band the
                // letters are cut into and belongs over it: the clock at one end of the case
                // and the battery at the other, both clear of the window between them.
                draw_status(
                    self.battery,
                    self.battery_percent,
                    self.bolt,
                    self.shelf_clock,
                    self.hud.face(Icon::of_mode(slot_ui::palette::mode())),
                    out,
                );
            }
            Phase::About { scroll, .. } => {
                // The same ground the shelf stands on, scrim and all. The label is a dark
                // object and the scrim is what a dark object needs to read over a
                // photograph — it is there for the carts for exactly the same reason.
                draw_backdrop(self.wallpaper, out);
                // One card, one scroll: the label goes up with the rows under it rather than
                // staying in the middle of the panel while the list moves behind it.
                draw_sticker_at(self.sticker_face, ABOUT_PAD - *scroll, out);
                let mut y = about_first_y() - *scroll;
                for face in self.shortcut_rows.iter().copied() {
                    // Off the panel is off the panel: twenty rows of which five are ever on
                    // screen is not a reason to hand the compositor twenty.
                    if y < OUT_H as f32 && y + face.2 as f32 > 0.0 {
                        draw_shortcut_row(Some(face), y, out);
                    }
                    y += face.2 as f32;
                }
                // Pinned. It is the line that says the rest of the card moves, so it cannot be
                // the thing that moves.
                if let Some((_, _, h)) = self.shortcut_hint {
                    // The band, then the line on it. The band is the panel's and the line is the
                    // card's, so they are laid from two different widths.
                    let top = OUT_H as f32 - h as f32;
                    draw_hint_bar(top, out);
                    draw_shortcut_row(self.shortcut_hint, top, out);
                }
                return;
            }
            Phase::Inserting { cart, resumed, .. } => {
                if !resumed {
                    draw_backdrop(self.wallpaper, out);
                    self.shelf()
                        .draw_row(Some(cart), 0.0, self.seat(), 1.0, out);
                }
                self.chrome(cart, self.seat(), out);
            }
            Phase::Ejecting { cart, .. } => {
                draw_backdrop(self.wallpaper, out);
                self.shelf()
                    .draw_row(Some(cart), 0.0, self.seat(), 1.0, out);
                self.chrome(cart, self.seat(), out);
            }
            Phase::Playing { cart } if self.screen < 1.0 => self.chrome(cart, 0.0, out),
            Phase::Playing { .. } => {
                self.push_game(out);
                self.draw_palette_browser(out);
            }
            // The paused game stays underneath, covered by the screenshot the switcher
            // draws over the whole screen.
            Phase::Polaroids { .. } => {
                self.push_game(out);
                if let Some(p) = &self.polaroids {
                    p.draw(
                        self.battery,
                        self.battery_percent,
                        self.bolt,
                        self.shelf_clock,
                        out,
                    );
                }
            }
            Phase::Doze { .. } => {
                out.push(Draw::Rect {
                    x: 0.0,
                    y: 0.0,
                    w: OUT_W as f32,
                    h: OUT_H as f32,
                    colour: [0.0, 0.0, 0.0, 1.0],
                });
                return;
            }
        }
        if let Some(picker) = self.core_picker_shown() {
            self.draw_core_picker(&picker, out);
        }
        if let Some(menu) = self.game_menu {
            self.draw_game_menu(menu, out);
        }
        // Over everything, in every phase. The bar is never what the user is looking at.
        //
        // The plate hangs under the letter band on the shelf and from the top of the screen
        // everywhere else, which is the one thing about the HUD a phase gets to decide: the band
        // is only on the shelf, and covering it with a plate would hide the index and the clock
        // for the second and a half a level is up.
        let plate_top = if band_on { PLATE_Y } else { 0.0 };
        self.hud.draw(self.now(), plate_top, out);
    }

    pub fn screen_shake(&self) -> f32 {
        self.shake_at(self.now())
    }

    pub fn shake_at(&self, now: Millis) -> f32 {
        self.shake_when(matches!(self.phase, Phase::Playing { .. }), now)
    }

    pub fn shelf_shake(&self) -> f32 {
        self.shake_when(self.on_shelf() && self.core_picker.is_none(), self.now())
    }

    fn shake_when(&self, mine: bool, now: Millis) -> f32 {
        if !mine {
            return 0.0;
        }
        self.refusal.map_or(0.0, |r| r.offset(now))
    }

    fn push_game(&self, out: &mut Vec<Draw>) {
        if self.game_visible() {
            // A tinted-backlight colour correction makes a GB picture glow one colour; the
            // letterbox around it would otherwise stay black and read as a frame that does not
            // belong. Fill it with the same colour, so the whole panel is one tinted surface.
            //
            // **Plain GB only.** GBC is a colour machine whose games use their own palettes, so
            // its surround keeps the black letterbox — a mono tint there would be a lie about
            // the machine, and GBC is themed through an overlay PNG instead. A GBA picture
            // fills the frame, so it has no letterbox to fill either. And only modes 3..=6 have
            // a single colour to fill with.
            if matches!(
                self.shelf.current_cart().map(|c| c.system()),
                Some(System::Gb)
            ) {
                if let Some(c) = self.display.cc_border(self.current_machine()) {
                    out.push(Draw::Rect {
                        x: 0.0,
                        y: 0.0,
                        w: OUT_W as f32,
                        h: OUT_H as f32,
                        colour: [c[0], c[1], c[2], 1.0],
                    });
                }
            }
            out.push(Draw::Game);
            // Over the game and under the HUD: a per-system screen overlay PNG from the card,
            // composited with its own alpha. Opaque pixels replace the picture (bezel, grid),
            // transparent ones let it through. No texture means the built-in look.
            if let Some(tex) = self.overlay_tex {
                out.push(Draw::Tex {
                    x: 0.0,
                    y: 0.0,
                    w: OUT_W as f32,
                    h: OUT_H as f32,
                    tex,
                    alpha: 1.0,
                });
            }
            // ...and the glow over the art, not under it: the art is the plastic's colour and
            // the glow is light landing on that plastic. Push it whether or not the card has an
            // overlay — the compositor is what knows whether anything reflective is up, and a
            // marker that costs one comparison is cheaper than asking the same question twice.
            out.push(Draw::Glow);
        }
    }

    fn chrome(&self, stem: &str, dim: f32, out: &mut Vec<Draw>) {
        let Some((cart, face)) = self.shelf().find(stem) else {
            return;
        };
        let alpha = self.alert_alpha();
        let (rest, scale) = self.shelf().selected_at();
        SlotChrome {
            cart,
            face,
            rest,
            scale,
            seat: self.seat(),
            alert: self.alert_face.filter(|_| alpha > 0.0).map(|t| (t, alpha)),
            dim,
            screen: self.screen,
            game: self.game_ready,
        }
        .draw(out);
    }

    pub fn alert_visible(&self) -> bool {
        self.alert_alpha() > 0.0
    }

    pub fn alert_alpha(&self) -> f32 {
        let (Some(from), Phase::Ejecting { t, .. }) = (self.refused_from, &self.phase) else {
            return 0.0;
        };
        let span = EJECT_S - from;
        if span <= 0.0 {
            return 0.0;
        }
        let u = ((t - from) / span).clamp(0.0, 1.0);
        ((ALERT_GONE - u) / (ALERT_GONE - ALERT_HOLD)).clamp(0.0, 1.0)
    }

    pub fn set_shelf_title_face(&mut self, face: Option<(TexId, u32, u32)>) {
        self.shelf_title_face = face;
    }

    pub fn set_alert_face(&mut self, face: TexId) {
        self.alert_face = Some(face);
    }

    pub fn set_shutdown_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.shutdown_faces = faces;
    }

    pub fn set_power_menu_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.power_menu_faces = faces;
    }

    pub fn set_core_board_faces(&mut self, board: TexId, lid: TexId) {
        self.core_board_face = Some(board);
        self.core_lid_face = Some(lid);
        self.core_faces_stem = self.selected_stem().map(str::to_string);
    }

    pub fn set_core_part_faces(
        &mut self,
        sockets: Vec<TexId>,
        chips: Vec<TexId>,
        blank: TexId,
        shadow: TexId,
    ) {
        self.core_socket_faces = sockets;
        self.core_chip_faces = chips;
        self.core_blank_chip_face = Some(blank);
        self.core_chip_shadow_face = Some(shadow);
    }

    pub fn set_core_legend_faces(&mut self, faces: Vec<(TexId, u32)>) {
        self.core_legend_faces = faces;
    }

    pub fn set_link_menu_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.link_menu_faces = faces;
    }

    pub fn set_link_linked_face(&mut self, face: (TexId, u32, u32)) {
        self.link_linked_face = Some(face);
    }

    pub fn set_link_step_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.link_step_faces = faces;
    }

    pub fn set_link_fail_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.link_fail_faces = faces;
    }

    pub fn set_link_legend_faces(&mut self, faces: Vec<(TexId, u32)>) {
        self.link_legend_faces = faces;
    }

    pub fn set_link_sprites(&mut self, sprites: LinkSprites) {
        self.link_sprites = Some(sprites);
    }

    pub fn link_sprites_ready(&self) -> bool {
        self.link_sprites.is_some()
    }

    fn draw_power_menu(&self, index: usize, out: &mut Vec<Draw>) {
        out.push(Draw::Rect {
            x: 0.0,
            y: 0.0,
            w: OUT_W as f32,
            h: OUT_H as f32,
            colour: slot_ui::opening(),
        });
        draw_menu_rows(
            &self.power_menu_faces,
            Some(index),
            centred_top(self.power_menu_faces.len()),
            out,
        );
    }

    /// The direct-connect dialog: a scrim over everything, then the lines for its stage centred on
    /// the panel. Stage 0 asks, stage 1 waits for the cable, stage 2 is the live mode.
    fn draw_direct_link(&self, stage: u8, out: &mut Vec<Draw>) {
        out.push(Draw::Rect {
            x: 0.0,
            y: 0.0,
            w: OUT_W as f32,
            h: OUT_H as f32,
            colour: slot_ui::opening(),
        });
        match stage {
            0 => {
                self.draw_centred_line(0, 200.0, out);
                self.draw_centred_line(1, 262.0, out);
            }
            1 => self.draw_centred_line(2, 240.0, out),
            _ => {
                self.draw_centred_line(3, 172.0, out);
                if let Some((tex, w, h)) = self.direct_ip_face {
                    out.push(Draw::Tex {
                        x: (OUT_W as f32 - w as f32) / 2.0,
                        y: 226.0,
                        w: w as f32,
                        h: h as f32,
                        tex,
                        alpha: 1.0,
                    });
                }
                self.draw_centred_line(4, 296.0, out);
            }
        }
    }

    /// One baked dialog line, centred horizontally on the panel at the given vertical centre.
    fn draw_centred_line(&self, i: usize, cy: f32, out: &mut Vec<Draw>) {
        if let Some(&(tex, w, h)) = self.direct_faces.get(i) {
            out.push(Draw::Tex {
                x: (OUT_W as f32 - w as f32) / 2.0,
                y: cy - h as f32 / 2.0,
                w: w as f32,
                h: h as f32,
                tex,
                alpha: 1.0,
            });
        }
    }

    /// The direct-connect lines, uploaded once at boot.
    pub fn set_direct_faces(&mut self, faces: Vec<(TexId, u32, u32)>) {
        self.direct_faces = faces;
    }

    /// Open the direct-connect prompt — a long hold of B on the shelf.
    fn open_direct_link(&mut self) {
        self.direct_link.get_or_insert(0);
    }

    /// A on the dialog: switch the gadget to mass storage. Nothing else happens on this stage.
    fn direct_confirm(&mut self) {
        if self.direct_link == Some(0) {
            self.direct_link = Some(1);
            self.direct_at = self.now();
            self.direct_ip = None;
            self.direct_ip_face = None;
            let _ = std::fs::remove_file("/tmp/slot-ip");
            self.run_direct_script(true);
        }
    }

    /// B on the dialog: cancel the prompt, or leave the live mode (drop the radio).
    fn direct_cancel(&mut self) {
        if self.direct_link == Some(2) {
            self.run_direct_script(false);
        }
        self.direct_link = None;
        self.direct_ip = None;
        self.direct_ip_face = None;
        self.direct_ip_dirty = false;
    }

    /// Run the enter/exit shell detached, so the dialog keeps drawing while the radio comes up.
    fn run_direct_script(&self, enter: bool) {
        let script = if enter { DIRECT_ENTER } else { DIRECT_EXIT };
        let _ = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{} >/dev/null 2>&1", script))
            .spawn();
    }

    /// The address the WiFi link came up on, and the frontend's bake hook for it. The dialog's own
    /// lines are baked once at boot, but this one is whatever DHCP handed out, so the frontend
    /// bakes it when `direct_ip_dirty` says it has arrived.
    pub fn direct_ip_dirty(&self) -> bool {
        self.direct_ip_dirty
    }

    pub fn direct_ip_str(&self) -> Option<&str> {
        self.direct_ip.as_deref()
    }

    pub fn set_direct_ip_face(&mut self, face: Option<(TexId, u32, u32)>) {
        self.direct_ip_face = face;
    }

    pub fn clear_direct_ip_dirty(&mut self) {
        self.direct_ip_dirty = false;
    }

    /// The picker, but only once it has started opening: `None` while it is still standing on
    /// the shelf waiting for this cart's faces, so nothing of it is on screen yet and the row
    /// has not made way for it.
    fn core_picker_shown(&self) -> Option<CorePicker> {
        self.core_picker.filter(|p| !p.waiting())
    }

    fn draw_core_picker(&self, picker: &CorePicker, out: &mut Vec<Draw>) {
        let now = self.now();
        let progress = picker.openness(now);
        let lift = lift_of(progress);
        let (rest, scale) = self.shelf().selected_at();
        let shelf = shelf_cart_at(rest, scale);
        let board = board_from(shelf, progress);
        let zoom = board_zoom(board);
        let ready = self.core_faces_ready();
        // A card that supplies its own board has the chips drawn into the art, so there is
        // nothing for the picker to place: no sockets, no chip, and no shadow for a chip that
        // is not coming across. What is left of the picker on such a card is the two lamps.
        let own_board = slot_ui::board_art().is_some();

        if ready {
            if let Some(tex) = self.core_board_face {
                out.push(Draw::Tex {
                    x: board.x,
                    y: board.y,
                    w: board.w,
                    h: board.h,
                    tex,
                    alpha: 1.0,
                });
            }
            // The card's board carries its own chips, so the lamps are the whole of what the
            // picker adds: the one the cursor is over lit orange, the other out, and the lit one
            // green once the choice has been made and the picker is on its way out. Nothing is
            // baked into the board texture, because the state changes every press.
            if let Some(art) = slot_ui::board_art() {
                let chip = picker.chip(now);
                let here = usize::from(chip.across >= 0.5);
                let done = picker.closing();
                for (i, lamp) in art.lamps.iter().enumerate() {
                    let colour = if i == here {
                        if done {
                            [0.18, 0.76, 0.37, 1.0]
                        } else {
                            [1.0, 0.55, 0.10, 1.0]
                        }
                    } else {
                        // Out, but not invisible: against the board's own plastic a dark lamp
                        // reads as a hole rather than as a lamp that is off.
                        [0.62, 0.62, 0.65, 1.0]
                    };
                    out.push(Draw::Rect {
                        x: board.x + lamp.x * board.w,
                        y: board.y + lamp.y * board.h,
                        w: lamp.w * board.w,
                        h: lamp.h * board.h,
                        colour,
                    });
                }
            }
            // The sockets are the **built-in** board's, and only its: a card that drew its own
            // board drew its chips into it as well, so placing these two on top leaves two empty
            // positions standing over a finished board — the picker's own furniture printed over
            // somebody else's drawing. The chip below is gated for the same reason and always
            // was; this loop was not, which is the pair of leftovers the player saw.
            if !own_board {
                // A face drawn at its own size is only sharp on whole pixels.
                for (i, tex) in self.core_socket_faces.iter().copied().enumerate() {
                    let (x, y) = on_board(board, SOCKET_U[i], SOCKET_V);
                    out.push(Draw::Tex {
                        x: x.round(),
                        y: y.round(),
                        w: SOCKET_W as f32 * zoom,
                        h: SOCKET_H as f32 * zoom,
                        tex,
                        alpha: 1.0,
                    });
                }
            }

            let chip = picker.chip(now);
            let u = CHIP_U[0] + (CHIP_U[1] - CHIP_U[0]) * chip.across;
            if !own_board && chip.lift > 0.0 {
                if let Some(tex) = self.core_chip_shadow_face {
                    let (cx, cy) = on_board(board, u + 19.0, CHIP_V + 29.4);
                    let (w, h) = (SHADOW_W as f32 * zoom, SHADOW_H as f32 * zoom);
                    out.push(Draw::Tex {
                        x: cx - w / 2.0,
                        y: cy - h / 2.0,
                        w,
                        h,
                        tex,
                        alpha: 0.6 * chip.lift * lift,
                    });
                }
            }
            // The blank stand-in is for a chip in flight, and there is no chip to fly across a
            // board that has its own: drawn anyway it would be a grey chip flashing over a
            // finished board, which is worse than nothing.
            let face = if own_board {
                None
            } else {
                match chip.seated {
                    Some(core) => self.core_chip_faces.get(core.index()).copied(),
                    None => self.core_blank_chip_face,
                }
            };
            if let Some(tex) = face {
                let (x, y) = on_board(board, u, CHIP_V - HOP_LIFT * chip.lift);
                let body = Placed {
                    x: x + chip.shake,
                    y,
                    w: CHIP_W as f32 * zoom,
                    h: CHIP_H as f32 * zoom,
                };
                let at = grown(body, TURN_PAD as f32 * zoom);
                out.push(Draw::Turned {
                    x: at.x.round(),
                    y: at.y.round(),
                    w: at.w,
                    h: at.h,
                    tex,
                    alpha: 1.0,
                    turn: chip.tip,
                });
            }
        }

        if let Some(tex) = self.core_chip_shadow_face {
            let (lid, _) = lid_from(shelf, progress);
            let k = lid.w / lid_at(1.0).0.w;
            let (w, h) = (LID_SHADOW_W * k, LID_SHADOW_H * k);
            out.push(Draw::Tex {
                x: lid.x + (lid.w - w) / 2.0,
                y: lid.y + lid.h + LID_SHADOW_DROP * k - h / 2.0,
                w,
                h,
                tex,
                alpha: LID_SHADOW_ALPHA * lift,
            });
        }

        if ready {
            if let Some(tex) = self.core_lid_face {
                let (lid, turn) = lid_from(shelf, progress);
                let at = grown(lid, TURN_PAD as f32 * lid.w / CART_W as f32);
                out.push(Draw::Turned {
                    x: at.x,
                    y: at.y,
                    w: at.w,
                    h: at.h,
                    tex,
                    alpha: 1.0,
                    turn,
                });
            }
        } else if let Some((_, Some(tex))) = self
            .selected_stem()
            .and_then(|stem| self.shelf().find(stem))
        {
            let (lid, turn) = lid_from(shelf, progress);
            out.push(Draw::Turned {
                x: lid.x,
                y: lid.y,
                w: lid.w,
                h: lid.h,
                tex,
                alpha: 1.0,
                turn,
            });
        }

        if let [cancel, swap, choose] = self.core_legend_faces.as_slice() {
            let right = BOARD_X + BOARD_W as f32;
            let seen = |w: u32| w.saturating_sub(HINT_EDGE) as f32;
            for (tex, w, x) in [
                (cancel.0, cancel.1, BOARD_X),
                (swap.0, swap.1, (OUT_W as f32 - seen(swap.1)) / 2.0),
                (choose.0, choose.1, right - seen(choose.1)),
            ] {
                out.push(Draw::Tex {
                    x: x.round(),
                    y: CORE_LEGEND_Y,
                    w: w as f32,
                    h: HINT_H as f32,
                    tex,
                    alpha: lift,
                });
            }
        }
    }

    fn draw_game_menu(&self, menu: GameMenu, out: &mut Vec<Draw>) {
        out.push(Draw::Rect {
            x: 0.0,
            y: 0.0,
            w: OUT_W as f32,
            h: OUT_H as f32,
            colour: slot_ui::opening(),
        });
        if let Some(sprites) = &self.link_sprites {
            crate::link_screen::draw_link_art(menu, self.link_hardware, self.now(), sprites, out);
        }
        let line = match menu {
            GameMenu::Pick(role) => self.link_menu_faces.get(role.index()).copied(),
            GameMenu::Working { step, .. } => self
                .link_step_faces
                .get(step.shown(self.radio.warmed()).index())
                .copied(),
            GameMenu::Linked { .. } => self.link_linked_face,
            GameMenu::Failed { fail, .. } => fail
                .shown()
                .and_then(|i| self.link_fail_faces.get(i).copied()),
            GameMenu::Unplug { .. } => None,
        };
        if let Some((tex, w, h)) = line {
            out.push(Draw::Tex {
                x: ((OUT_W as f32 - w as f32) / 2.0).round(),
                y: LINK_TEXT_Y,
                w: w as f32,
                h: h as f32,
                tex,
                alpha: 1.0,
            });
        }
        let switchable = self.seated().is_some_and(|stem| self.link_switchable(stem));
        let keys: &[LinkLegend] = match menu {
            GameMenu::Pick(_) if switchable => &[
                LinkLegend::Cancel,
                LinkLegend::Mode,
                LinkLegend::Swap,
                LinkLegend::Link,
            ],
            GameMenu::Pick(_) => &[LinkLegend::Cancel, LinkLegend::Swap, LinkLegend::Link],
            GameMenu::Working { .. } => &[LinkLegend::Cancel],
            GameMenu::Linked { opened: true, .. } => &[LinkLegend::Back, LinkLegend::EndLink],
            GameMenu::Linked { .. } => &[],
            GameMenu::Failed { .. } => &[LinkLegend::Ok],
            GameMenu::Unplug { .. } => &[],
        };
        let faces: Vec<(TexId, u32)> = keys
            .iter()
            .filter_map(|k| self.link_legend_faces.get(k.index()).copied())
            .collect();
        let seen = |w: u32| w.saturating_sub(HINT_EDGE) as f32;
        let total: f32 = faces.iter().map(|(_, w)| seen(*w)).sum::<f32>()
            + LINK_LEGEND_GAP * faces.len().saturating_sub(1) as f32;
        let mut x = ((OUT_W as f32 - total) / 2.0).round();
        for (tex, w) in faces {
            out.push(Draw::Tex {
                x,
                y: LINK_LEGEND_Y,
                w: w as f32,
                h: HINT_H as f32,
                tex,
                alpha: 1.0,
            });
            x += (seen(w) + LINK_LEGEND_GAP).round();
        }
    }

    fn auto_link(&self, stem: &str) -> Option<(&Cart, LinkKind)> {
        let cart = self.shelf().carts.iter().find(|c| c.stem == stem)?;
        let auto = link_kind(&cart.code, &cart.title, slot_store::header_clean(&cart.rom));
        Some((cart, auto))
    }

    fn on_shelf(&self) -> bool {
        matches!(self.phase, Phase::Shelf)
    }

    pub fn browsed_rom(&self) -> Option<&std::path::Path> {
        if !self.on_shelf() {
            return None;
        }
        let shelf = self.shelf();
        shelf.carts.get(shelf.index).map(|c| c.rom.as_path())
    }

    fn insert(&mut self, clean: bool) {
        if !self.on_shelf() {
            return;
        }
        let Some(i) = self.shelf.current() else {
            return;
        };
        self.shelf.carts[i].read_header();
        let cart = self.shelf.carts[i].stem.clone();
        eprintln!(
            "slot: launch: insert {cart} at {:.1} ms",
            crate::boot_time::ms()
        );
        self.play_held = None;
        self.refusal = None;
        self.refused_from = None;
        self.phase = Phase::Inserting {
            cart,
            t: 0.0,
            core_ready: false,
            resumed: false,
            clean,
        };
    }

    pub fn starting_clean(&self) -> bool {
        matches!(self.phase, Phase::Inserting { clean: true, .. })
    }

    fn play_hold(&mut self) {
        let Some(at) = self.play_held else {
            return;
        };
        if !self.on_shelf() {
            self.play_held = None;
            return;
        }
        if self.now().saturating_sub(at) >= PLAY_HOLD_MS {
            self.insert(true);
        }
    }

    /// The other hold on this screen. Spent rather than left for the release, because the
    /// shelf stays up afterwards and a hold that kept firing every timer tick would swap the
    /// view over and over for as long as the button was down.
    fn fav_hold(&mut self) {
        let Some(at) = self.fav_held else {
            return;
        };
        if !self.on_shelf() {
            self.fav_held = None;
            return;
        }
        if self.now().saturating_sub(at) >= PLAY_HOLD_MS {
            self.fav_held = None;
            self.toggle_fav_view();
        }
    }

    /// A hold of Y: swap which machine's shelf is up. Spent like `fav_hold`, so it fires once
    /// rather than on every timer tick the button stays down.
    fn sys_hold(&mut self) {
        let Some(at) = self.sys_held else {
            return;
        };
        if !self.on_shelf() {
            self.sys_held = None;
            return;
        }
        if self.now().saturating_sub(at) >= PLAY_HOLD_MS {
            self.sys_held = None;
            self.cycle_system();
        }
    }

    /// A hold of B: open the direct-connect dialog. Spent like `fav_hold`.
    fn mtp_hold(&mut self) {
        let Some(at) = self.mtp_held else {
            return;
        };
        if !self.on_shelf() {
            self.mtp_held = None;
            return;
        }
        if self.now().saturating_sub(at) >= PLAY_HOLD_MS {
            self.mtp_held = None;
            self.open_direct_link();
        }
    }

    /// A short X on the shelf: star the cart under the caret, or take the star back.
    /// The star is the whole of the feedback — it appears on the cart, which is where the
    /// user is already looking.
    fn toggle_favorite(&mut self) {
        let Some(stem) = self.shelf.current_cart().map(|c| c.stem.clone()) else {
            return;
        };
        if !self.state.favorites.remove(&stem) {
            self.state.favorites.insert(stem);
        }
        self.persist();
        // The favourites shelf shows the starred carts and nothing else, so a cart that just
        // lost its star has to leave the view it is standing in.
        if self.fav_view {
            self.show_favorites();
        }
    }

    /// A hold of X: swap between the whole of the current machine's shelf and its starred
    /// carts. The library is three machines and only one shelf is up, so "the whole shelf" is
    /// this machine's carts, not all of them.
    fn toggle_fav_view(&mut self) {
        if self.fav_view {
            self.show_all();
        } else {
            self.show_favorites();
        }
    }

    /// The whole of the machine whose shelf is up — what leaving the favourites sub-view
    /// returns to.
    fn show_all(&mut self) {
        let keep = self.shelf.current_cart().map(|c| c.stem.clone());
        let view = self.system_view(self.shelf_system);
        self.fav_view = false;
        self.shelf.set_view(view, keep.as_deref());
        self.retally_letters();
    }

    /// A tap of Y on the shelf: step the current machine's subfolder filter — all, then each of
    /// its folders, then all again. A machine whose roms all sit loose in its own folder has
    /// nothing to step, and the press is refused rather than dropped, so it reads as answered.
    fn cycle_folder(&mut self) {
        if !self.shelf.cycle_folder(self.shelf_system) {
            return self.refuse();
        }
        let keep = self.shelf.current_cart().map(|c| c.stem.clone());
        let view = self.system_view(self.shelf_system);
        self.shelf.set_view(view, keep.as_deref());
        self.retally_letters();
        // The view is a different set of carts, so the resident window has to be rebuilt the way
        // a machine switch rebuilds it: the screen would otherwise fill with stand-ins.
        self.shelf_dirty = true;
    }

    /// The starred carts of the machine whose shelf is up, in library order so a cart sits on
    /// the same shelf, at the same place among its neighbours, as the one it came from.
    /// Refused rather than shown empty when nothing is starred: an empty shelf is a screen with
    /// nothing on it and no way to tell why.
    fn show_favorites(&mut self) {
        let keep = self.shelf.current_cart().map(|c| c.stem.clone());
        let view: Vec<usize> = self
            .shelf
            .carts
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.system() == self.shelf_system && self.state.favorites.contains(&c.stem)
            })
            .map(|(i, _)| i)
            .collect();
        if view.is_empty() {
            return self.refuse();
        }
        self.fav_view = true;
        self.shelf.set_view(view, keep.as_deref());
        self.retally_letters();
    }

    /// The letter dial counts carts, so a view that is not the whole library has its own
    /// tally. Rebuilt on the edge rather than per frame: the card cannot change while slot is
    /// running, so the only thing that can move it is a view swap.
    fn retally_letters(&mut self) {
        let visible: Vec<Cart> = self.shelf.visible().into_iter().cloned().collect();
        self.letter_counts = tally_letters(&visible);
    }

    /// The star over every starred cart in the row.
    ///
    /// **Inside the cart**, in the moulded band the shell leaves between its curved top edge and
    /// the top of the label — `cart::label_panel` puts the label at 22.8% of the face, so that
    /// band is the top fifth of the shell. Directly above the sticker and still on the cartridge,
    /// under the arc, rather than floating over the row: the star is printed on the plastic, not
    /// pinned above the object. Scaled with the cart, so the enlarged selection wears a larger
    /// star and the row still reads as one row.
    fn draw_fav_stars(&self, out: &mut Vec<Draw>) {
        /// How tall the mark is on a cart, in screen pixels. Absolute rather than a share of the
        /// cart, because it is a reading and not part of the artwork: a star that grew with the
        /// selected cart would say the cart was more of a favourite than its neighbours.
        const STAR_PX: f32 = 20.0;
        let Some((tex, w, h)) = self.fav_star else {
            return;
        };
        if self.state.favorites.is_empty() {
            return;
        }
        let (w, h) = (w as f32, h as f32);
        for r in self.shelf.slot_rects(None, self.shelf_shake(), 0.0) {
            if !self
                .state
                .favorites
                .contains(&self.shelf.carts[r.cart].stem)
            {
                continue;
            }
            let (sh, sw) = (STAR_PX, w * (STAR_PX / h));
            // Centred on the label's top right corner: the corner of the paper, which is the one
            // piece of the cart that is the same on every machine's artwork and the place the eye
            // already goes. A card that supplies its own art says where its label is — the
            // magenta slot the label is actually fitted into — and that answer has to be used,
            // because it is not the built-in panel. Without one, the panel stands in.
            let corner = slot_ui::cart_label_share(self.shelf.system())
                .map(|(_, y0, x1, _)| (r.x + x1 * r.w, r.y + y0 * r.h))
                .unwrap_or_else(|| {
                    let (_, y0, x1, _) = self.shelf.size().label_panel_at(r.w as u32, r.h as u32);
                    (r.x + x1 as f32, r.y + y0 as f32)
                });
            let x = corner.0 - sw / 2.0;
            let y = corner.1 - sh / 2.0;
            // A bloom under it: the same star, larger and faint, so it reads as light coming
            // off the glyph rather than as a second outline. One extra quad, and the linear
            // tap on an upscaled alpha map is the blur — no pass and no blur shader.
            let g = sh * 0.45;
            out.push(Draw::Tex {
                x: x - g / 2.0,
                y: y - g / 2.0,
                w: sw + g,
                h: sh + g,
                tex,
                alpha: r.alpha * 0.30,
            });
            out.push(Draw::Tex {
                x,
                y,
                w: sw,
                h: sh,
                tex,
                alpha: r.alpha,
            });
        }
    }

    /// Which shelf is up, in the bottom right corner of the screen — where the original printed
    /// the time: hollow while the ordinary shelf is showing, solid and yellow while the
    /// favourites are. A reading rather than a control — the hold of B is the control — so it
    /// never looks like something to press.
    ///
    /// The mode badge (sun in light, moon in dark) is not here: it is printed beside the clock at
    /// the top of the band, where it reads as part of the same row of type.
    fn draw_fav_indicator(&self, out: &mut Vec<Draw>) {
        let lit = self.fav_view;
        let Some((tex, w, h)) = (if lit {
            self.fav_ind.map(|p| p.1)
        } else {
            self.fav_ind.map(|p| p.0)
        }) else {
            return;
        };
        if h == 0 {
            return;
        }
        let k = FAV_IND_DRAW / h as f32;
        let (sw, sh) = (w as f32 * k, FAV_IND_DRAW);
        // Centred on the battery reading's own centre at the other end of the top band, so the two
        // right-hand marks stand in one column.
        let read_right = OUT_W as f32 - STATUS_READ_MARGIN;
        let read_cx = read_right - self.battery_percent.w as f32 / 2.0;
        let ind_x = read_cx - sw / 2.0;
        let y = OUT_H as f32 - MOUTH_H + MOUTH_H / 2.0 - sh / 2.0;
        out.push(Draw::Tex {
            x: ind_x,
            y,
            w: sw,
            h: sh,
            tex,
            alpha: 1.0,
        });
    }

    fn eject(&mut self) {
        if self.single_cart() {
            return self.refuse();
        }
        let cart = match &mut self.phase {
            Phase::Playing { cart } | Phase::Inserting { cart, .. } => std::mem::take(cart),
            _ => return,
        };
        self.end_link();
        self.close_game_menu();
        self.close_cheat_menu();
        self.close_palette_browser();
        self.flush_eject(&cart);
        self.pending = None;
        self.refusal = None;
        self.refused_from = None;
        self.phase = Phase::Ejecting {
            cart,
            t: -EJECT_HOLD_S,
        };
    }

    fn flush_eject(&mut self, stem: &str) {
        self.settle_saves();
        let (Some(root), Some(snapshot)) = (&self.root, &self.snapshot) else {
            return;
        };
        let Some(state) = snapshot.state() else {
            eprintln!("slot: eject: the core gave up no state");
            return;
        };
        let Some(system) = self.system_of(stem) else {
            eprintln!("slot: eject: no cart in the shelf for {stem}");
            return;
        };
        let (state, sav) = trusted_write(snapshot.as_ref(), state, "eject");
        let state = state.filter(|_| self.state.eject_save);
        match persist::eject(
            root,
            self.platform,
            self.core,
            system,
            stem,
            state.as_deref(),
            sav.as_deref(),
        ) {
            Ok(()) => {
                self.state.cart = None;
                self.state.cart_platform = None;
                self.card_failing.store(false, Ordering::Relaxed);
            }
            Err(e) => {
                eprintln!("slot: eject: {e}");
                self.card_failing.store(true, Ordering::Relaxed);
            }
        }
    }

    fn doze(&mut self) {
        if self.link_active() {
            self.end_link();
        }
        self.close_game_menu();
        self.close_cheat_menu();
        self.close_palette_browser();
        // A shut lid is walking away, not choosing. Nothing is written and nothing animates:
        // waking comes back to a plain shelf.
        self.core_picker = None;
        if matches!(self.phase, Phase::Doze { .. }) {
            return;
        }
        self.state_writer.flush();
        self.flush_resume();
        let cart = match &mut self.phase {
            Phase::Playing { cart } | Phase::Polaroids { cart } => Some(std::mem::take(cart)),
            _ => None,
        };
        self.polaroids = None;
        self.phase = Phase::Doze { cart };
        self.dozed_at = self.now();
        self.radio.ask(RadioJob::Cool);
        if let Some(power) = &mut self.power {
            power.on_close();
        }
    }

    fn wake(&mut self) {
        let Phase::Doze { cart } = &mut self.phase else {
            return;
        };
        self.phase = match cart.take() {
            Some(cart) => Phase::Playing { cart },
            None => Phase::Shelf,
        };
        if let Some(power) = &mut self.power {
            power.on_open();
        }
    }

    pub fn on_doze_timeout(&mut self) {
        if !matches!(self.phase, Phase::Doze { .. }) {
            return;
        }
        self.begin_power_off();
    }

    fn power_press(&mut self) {
        if std::mem::take(&mut self.woke_on_press) {
            return;
        }
        match self.phase {
            Phase::Doze { .. } => self.wake(),
            _ => self.doze(),
        }
    }

    fn open_power_menu(&mut self) {
        if self.power_menu.is_some() {
            return;
        }
        if self.link_active() {
            return self.refuse();
        }
        self.flush_resume();
        self.power_menu = Some(0);
    }

    fn power_menu_input(&mut self, action: Action) {
        let Some(index) = self.power_menu else {
            return;
        };
        let last = PowerChoice::ALL.len() - 1;
        match action {
            Action::GbaDown(Btn::Up) => {
                self.power_menu = Some(if index == 0 { last } else { index - 1 })
            }
            Action::GbaDown(Btn::Down) => {
                self.power_menu = Some(if index >= last { 0 } else { index + 1 })
            }
            Action::GbaDown(Btn::B) => self.power_menu = None,
            Action::GbaDown(Btn::A) => {
                self.power_menu = None;
                self.close_game_menu();
                match PowerChoice::ALL[index] {
                    PowerChoice::Restart => {
                        self.restarting = true;
                        self.act_at = self.now() + SHUTDOWN_SHOW_MS;
                        self.set_led(LedState::Off);
                    }
                    PowerChoice::PowerOff => self.begin_power_off(),
                }
            }
            _ => {}
        }
    }

    /// The seated cart's cheat codes with their live on/off, in file order. Read by the
    /// frontend to rasterise the table's row faces and by the draw to colour each chip.
    pub(crate) fn cheats(&self) -> &[CheatItem] {
        &self.cheats
    }

    /// Replace the cheat list (called when a cart's core spawns) and close any open table, so a
    /// switch to a cart with none does not leave the highlight pointing at the old codes.
    pub(crate) fn set_cheats(&mut self, entries: Vec<crate::root::CheatEntry>) {
        self.cheats = entries
            .into_iter()
            .map(|e| CheatItem {
                code: e.code,
                desc: e.desc,
                // Default every code OFF: the list opens with nothing applied, and the
                // player switches on only the codes they want. The master `cheats_on`
                // stays on so a per-row toggle here still takes effect when pushed.
                enabled: false,
            })
            .collect();
        self.cheat_menu = None;
    }

    /// `Some` while the cheat table is up, `None` otherwise. Read by `Session` to pause the core
    /// and to decide what SELECT+A does: open it, or master-toggle-all while it is open.
    pub fn cheat_menu(&self) -> Option<usize> {
        self.cheat_menu
    }

    /// Whether the table's per-row edit needs pushing to the core. Cleared by `Session` once it
    /// has re-sent the list, because the table cannot reach the emulator itself.
    pub(crate) fn take_cheats_dirty(&mut self) -> bool {
        std::mem::take(&mut self.cheats_dirty)
    }

    /// Whether the mode moved since the last frame, and clears it. The binary takes this every
    /// frame the way it takes `take_cheats_dirty`: the palette is a global and the textures are
    /// not, so the one thing that has to be told is whoever owns the textures.
    pub(crate) fn take_mode_dirty(&mut self) -> bool {
        std::mem::take(&mut self.mode_dirty)
    }

    /// Whether a shelf swap has left the row's frame cut for the wrong machine. Taken once by
    /// the frontend, which has the only compositor.
    pub(crate) fn take_shelf_dirty(&mut self) -> bool {
        std::mem::take(&mut self.shelf_dirty)
    }

    /// SELECT+START: print the device the other way round.
    ///
    /// Three things, and the order is the order they have to happen in. The palette moves
    /// first, so that every quad drawn from here on — this frame included — is already the new
    /// colour. The state file follows, so a device switched off a second later comes back the
    /// way it was left. And the flag goes up last, because it is a request to someone else and
    /// nothing about this frame depends on the answer.
    ///
    /// The mode is a *setting* rather than a screen, and it is written through the same door
    /// the levels use, so a device switched off a second after the press comes back the way it
    /// was left rather than the way it booted.
    pub(crate) fn toggle_mode(&mut self) {
        let mode = slot_ui::palette::mode().other();
        slot_ui::palette::set_mode(mode);
        // A theme change repaints everything the palette touches: the fixed furniture (icons,
        // favourite stars, machine logos), the lazy caches (shelf title, clock, charge, about,
        // switcher), and the cart faces — which bake the shell colour in light mode
        // (`housing()`). `mode_dirty` pokes the frontend, which re-runs `rebake` in place on the
        // next frame: it re-rasterises the fixed faces and the visible shelf window, and drops
        // the rest so the resident pump rebuilds each as the caret reaches it. That is the whole
        // job a restart did, without re-running the boot — so no restart here.
        self.state.mode = mode;
        self.mode_dirty = true;
        self.persist();
    }

    /// SELECT+A over a running game. Opens the table — the "see the whole list and pick" screen
    /// a bare ON/OFF toast never was. The same chord again, while the table is up, flips every
    /// code at once: the old master toggle, now reachable from inside.
    pub(crate) fn open_cheat_menu(&mut self) {
        if self.cheat_menu.is_some() {
            return;
        }
        // The table is the only overlay that should be up while it is; an open in-game menu would
        // otherwise sit underneath it and steal nothing but still be half-true.
        self.game_menu = None;
        self.cheat_menu = Some(0);
    }

    /// Closes the table without touching any code's state.
    fn close_cheat_menu(&mut self) {
        self.cheat_menu = None;
    }

    /// `Some(cell)` while the palette browser is up, `None` otherwise. Read by `Session` to pause
    /// the core and to keep the pad out of it, the same way it reads the cheat table.
    pub fn palette_browser(&self) -> Option<usize> {
        self.pal_cursor
    }

    /// Hold of the palette key: the browser, on the page the key is walking, with the caret on the
    /// palette that is actually up.
    ///
    /// A Game Boy's list is hundreds of entries and this is the only way to see them; a colour
    /// machine's is eighteen, which is two pages and the reason it has a browser at all rather
    /// than a ring of eighteen presses. Only while a game is running, too: there is no picture on
    /// the shelf to recolour, and a hold there is simply nothing.
    ///
    /// **Refused while a model is up**, for the reason `cycle_cc` gives: a model's four shades are
    /// bound to its screen art, so a browser that offered to recolour the picture inside that case
    /// would be offering something it could not then do. Refused rather than dropped, so the press
    /// reads as answered.
    fn open_palette_browser(&mut self) {
        if self.pal_cursor.is_some() {
            return;
        }
        if !matches!(self.phase, Phase::Playing { .. }) {
            return;
        }
        // A model's art is bound to its shades, on a Game Boy; nothing else here refuses.
        if self.current_machine() == System::Gb
            && App::model_of_index(self.overlay_slot, self.overlay_index).is_some()
        {
            self.refuse();
            return;
        }
        // The page and the caret both come from the entry actually in hand, so the browser opens
        // on the one that is up. Deriving the page rather than reading `pal_page` is what keeps a
        // colour machine's two pages of nine away from the Game Boy's page count of its own
        // hundreds-long table — the two are not the same number and never were.
        let machine = self.current_machine();
        let per = App::grades_per_page(machine);
        let at = self.display.cc[DisplayFilter::machine_slot(machine)] as usize;
        self.pal_page = at / per;
        self.pal_cursor = Some(at.saturating_sub(self.pal_page * per));
    }

    /// How many entries the browser's table has for the machine that is up.
    fn browser_total(&self, machine: System) -> usize {
        if machine == System::Gb {
            crate::palettes::GB_PALETTES.len()
        } else {
            GRADES.len()
        }
    }

    /// The name of one of those entries, and the swatch its cell is painted with: a Game Boy's own
    /// four shades, or the grade walked from black to white (`grade_swatch`).
    fn browser_entry(&self, machine: System, at: usize) -> (&'static str, slot_ui::PaletteSwatch) {
        if machine == System::Gb {
            let e = &crate::palettes::GB_PALETTES[at.min(crate::palettes::GB_PALETTES.len() - 1)];
            (e.0, slot_ui::PaletteSwatch::shades(e.1))
        } else {
            let g = &GRADES[at.min(GRADES.len() - 1)];
            (g.name, grade_swatch(g))
        }
    }

    fn close_palette_browser(&mut self) {
        self.pal_cursor = None;
    }

    /// The browser owns every button on the game's side while it is up. The arrows ring inside the
    /// page — a row down is three cells — L and R turn the page, A takes the palette under the
    /// caret and B leaves without it.
    ///
    /// The caret rings rather than stops at the edges because the page is what bounds it, and L/R
    /// are the way out; a caret that could reach an edge and stop would be a caret the user has to
    /// find their way back from.
    fn palette_browser_input(&mut self, action: Action) {
        let Some(cursor) = self.pal_cursor else {
            return;
        };
        let machine = self.current_machine();
        let total = self.browser_total(machine);
        let per = App::grades_per_page(machine);
        match action {
            Action::GbaDown(Btn::Left) | Action::ShelfLeft => {
                self.pal_cursor = Some(slot_store::step_cursor(self.pal_page, cursor, -1, total, per));
            }
            Action::GbaDown(Btn::Right) | Action::ShelfRight => {
                self.pal_cursor = Some(slot_store::step_cursor(self.pal_page, cursor, 1, total, per));
            }
            Action::GbaDown(Btn::Up) => {
                self.pal_cursor = Some(slot_store::step_cursor(self.pal_page, cursor, -3, total, per));
            }
            Action::GbaDown(Btn::Down) => {
                self.pal_cursor = Some(slot_store::step_cursor(self.pal_page, cursor, 3, total, per));
            }
            Action::GbaDown(Btn::L1) | Action::GbaDown(Btn::R1) => {
                // The caret is a cell on the old page, so a shorter page it arrives on clamps it
                // rather than dropping it off the end. Only the last page is short, and only by
                // thirteen cells, so this is one press of the boundary rather than a rule.
                let delta = if action == Action::GbaDown(Btn::L1) {
                    -1
                } else {
                    1
                };
                self.pal_page = slot_store::step_page(self.pal_page, delta, total, per);
                let n = slot_store::len_on(self.pal_page, total, per);
                self.pal_cursor = Some(cursor.min(n.saturating_sub(1)));
            }
            Action::GbaDown(Btn::A) => {
                let picked = slot_store::index_of(self.pal_page, cursor, total, per);
                self.display.cc[DisplayFilter::machine_slot(machine)] = picked as u16;
                if let Some(root) = &self.root {
                    crate::root::write_display_modes(root, self.display.mask_mode, self.display.cc);
                }
                self.close_palette_browser();
            }
            Action::GbaDown(Btn::B) => self.close_palette_browser(),
            _ => {}
        }
    }

    /// What the browser's row of names is keyed on: the machine and the page that is up, or
    /// `None` while it is shut.
    ///
    /// A key rather than the names themselves, because a page's names are a compile-time fact
    /// about a table that never changes: the frontend rebuilds its fifteen faces when this moves
    /// and at no other time, so a caret walking inside a page costs nothing at all.
    pub fn palette_names_key(&self) -> Option<(System, usize)> {
        self.pal_cursor.map(|_| (self.current_machine(), self.pal_page))
    }

    /// The names of the entries on that page, in table order — one per cell, so every cell says
    /// what it is rather than only the one the caret is on. As long as the page and no longer:
    /// the last page of a table is the short one.
    ///
    /// **Empty on a Game Boy, and that is the whole of why this can be empty.** A cell is 196
    /// pixels wide and a colour machine's nine grades are named in four to six characters apiece,
    /// so they fit under their own cells and the page can be read across. A Game Boy's 343
    /// palettes are named by the core and the names are *long*: measured in the face the machine
    /// carries, 263 of them are wider than a cell at the type size the grades are set in, and the
    /// longest is still 30 pixels too wide at ten. Those pages keep the one line at the foot of
    /// the panel (`palette_name_want`), which has 640 pixels to work in and so never cuts a name.
    pub fn palette_names(&self) -> Vec<String> {
        let machine = self.current_machine();
        if machine == System::Gb {
            return Vec::new();
        }
        let per = App::grades_per_page(machine);
        let total = self.browser_total(machine);
        let base = (self.pal_page * per).min(total);
        let n = slot_store::len_on(self.pal_page, total, per);
        (0..n)
            .map(|i| self.browser_entry(machine, base + i).0.to_string())
            .collect()
    }

    /// The faces the frontend built for that page, in the same order.
    pub(crate) fn set_palette_names(&mut self, names: Vec<(TexId, u32, u32)>) {
        self.pal_names = names;
    }

    /// The name of the entry under the browser's caret, or `None` while it is shut **or on a
    /// machine whose cells are named** — the two readings are exclusive, and neither machine has
    /// both. The frontend mints a face from this and nothing else; comparing it to the last one it
    /// built is what keeps a moved caret to one rasterise and a still one to none.
    pub fn palette_name_want(&self) -> Option<String> {
        let cursor = self.pal_cursor?;
        let machine = self.current_machine();
        if machine != System::Gb {
            return None;
        }
        let per = App::grades_per_page(machine);
        let total = self.browser_total(machine);
        let at = slot_store::index_of(self.pal_page, cursor, total, per);
        Some(self.browser_entry(machine, at).0.to_string())
    }

    /// The face the frontend built for that name.
    pub(crate) fn set_palette_name_face(&mut self, face: Option<(TexId, u32, u32)>) {
        self.pal_name = face;
    }

    /// The palette browser, over the paused game. Only ever up over a cart in the slot, so the
    /// `Phase::Playing` arm of `draw` is the only caller.
    fn draw_palette_browser(&self, out: &mut Vec<Draw>) {
        let Some(cursor) = self.pal_cursor else {
            return;
        };
        let machine = self.current_machine();
        let per = App::grades_per_page(machine);
        let total = self.browser_total(machine);
        let base = (self.pal_page * per).min(total);
        let n = slot_store::len_on(self.pal_page, total, per);
        // A copy of the page's swatches, at most fifteen of them and only while the browser is
        // open. A Game Boy's entries carry their own four shades; a colour grade's cell is the
        // grade itself walked from black to white (`grade_swatch`).
        let swatches: Vec<slot_ui::PaletteSwatch> = (base..base + n)
            .map(|at| self.browser_entry(machine, at).1)
            .collect();
        let view = slot_ui::PaletteView {
            page: self.pal_page,
            cursor,
            pages: slot_ui::palette_pages(total, per),
            rows: slot_ui::palette_rows(per),
            swatches: &swatches,
            names: &self.pal_names,
            foot: self.pal_name,
        };
        slot_ui::draw_palette_browser(&view, out);
    }

    /// The master switch behind SELECT+A's second press: flips the lot on or off at once.
    pub(crate) fn toggle_all_cheats(&mut self) {
        self.cheats_on = !self.cheats_on;
    }

    /// The table owns every button on the game's side while it is up. Up and down move, A flips
    /// the highlighted code, B leaves. The master chord is handled in `Session::act`, before this
    /// sees the action, so it never arrives here as a toggle.
    fn cheat_menu_input(&mut self, action: Action) {
        let Some(index) = self.cheat_menu else {
            return;
        };
        let last = self.cheats.len().saturating_sub(1);
        match action {
            Action::GbaDown(Btn::Up) => self.cheat_menu = Some(index.saturating_sub(1)),
            Action::GbaDown(Btn::Down) => self.cheat_menu = Some((index + 1).min(last)),
            Action::GbaDown(Btn::A) => {
                if let Some(item) = self.cheats.get_mut(index) {
                    item.enabled = !item.enabled;
                }
                self.cheats_dirty = true;
            }
            Action::GbaDown(Btn::B) => self.close_cheat_menu(),
            _ => {}
        }
    }

    /// The "no cheats" panel, rasterised once at boot. Shown by `draw_cheat_menu` when the table
    /// opens on a cart that carries no codes.
    pub(crate) fn set_cheat_empty_face(&mut self, face: (TexId, u32, u32)) {
        self.cheat_empty_face = Some(face);
    }

    /// (Re)bind the row faces to a cart's codes, with the stem they belong to so the frontend
    /// knows when to rebuild. Called from the frontend's per-frame sync.
    pub(crate) fn set_cheat_faces(&mut self, stem: Option<String>, faces: Vec<(TexId, u32, u32)>) {
        self.cheat_face_stem = stem;
        self.cheat_faces = faces;
    }

    /// The stem whose faces `cheat_faces` currently holds. `None` until the first cart with
    /// cheats seats, and again after eject.
    pub(crate) fn cheat_face_stem(&self) -> Option<&str> {
        self.cheat_face_stem.as_deref()
    }

    /// How many row faces `cheat_faces` currently holds. Compared against `cheats().len()` so the
    /// frontend knows to rebuild after the codes load from the card post-spawn.
    pub(crate) fn cheat_face_count(&self) -> usize {
        self.cheat_faces.len()
    }

    /// The cheat table: a dark panel over the paused game, listing every code with a green (on)
    /// or grey (off) chip, the highlighted row carried on the menu's own bar. A window of
    /// `CHEAT_VISIBLE` rows scrolls so the highlight stays on screen no matter how long the list.
    fn draw_cheat_menu(&self, index: usize, out: &mut Vec<Draw>) {
        out.push(Draw::Rect {
            x: 0.0,
            y: 0.0,
            w: OUT_W as f32,
            h: OUT_H as f32,
            colour: slot_ui::opening(),
        });
        if self.cheats.is_empty() {
            if let Some((tex, w, h)) = self.cheat_empty_face {
                out.push(Draw::Tex {
                    x: ((OUT_W - w) / 2) as f32,
                    y: ((OUT_H - h) / 2) as f32,
                    w: w as f32,
                    h: h as f32,
                    tex,
                    alpha: 1.0,
                });
            }
            return;
        }
        let n = self.cheats.len();
        let vis = n.min(CHEAT_VISIBLE);
        let start = if n <= CHEAT_VISIBLE {
            0
        } else {
            index
                .saturating_sub(CHEAT_VISIBLE / 2)
                .min(n - CHEAT_VISIBLE)
        };
        let top = centred_top(vis);
        let left: f32 = 56.0;
        for v in 0..vis {
            let row = start + v;
            if row >= n {
                break;
            }
            let y = top + CHEAT_PITCH * v as f32;
            if row == index {
                out.push(Draw::Rect {
                    x: 8.0,
                    y: y + POWER_MENU_BAR_INSET,
                    w: (OUT_W - 16) as f32,
                    h: CHEAT_PITCH - 2.0 * POWER_MENU_BAR_INSET,
                    colour: slot_ui::edge(),
                });
            }
            // ON/OFF chip to the left of the code: green when live, grey otherwise. "Live" means
            // the code is switched on AND the master is on.
            let on = self.cheats_on && self.cheats[row].enabled;
            let iw = 14.0;
            let ix = 24.0;
            let iy = y + (CHEAT_PITCH - iw) / 2.0;
            out.push(Draw::Rect {
                x: ix,
                y: iy,
                w: iw,
                h: iw,
                colour: if on {
                    [0.30, 0.85, 0.45, 1.0]
                } else {
                    [0.55, 0.55, 0.6, 1.0]
                },
            });
            if let Some((tex, w, h)) = self.cheat_faces.get(row).copied() {
                out.push(Draw::Tex {
                    x: left,
                    y: y + (CHEAT_PITCH - h as f32) / 2.0,
                    w: w as f32,
                    h: h as f32,
                    tex,
                    alpha: 1.0,
                });
            }
        }
    }

    /// SELECT on the shelf offers the highlighted cart's core, opening on the one it already
    /// uses so the menu answers "which is this?" before it asks "which do you want?".
    ///
    /// Both the read that positions the highlight and the write that follows need the card.
    /// Without one there is nothing to configure and nowhere to put an answer, so the button
    /// stays inert rather than raising a menu whose choice would evaporate.
    fn open_core_picker(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        // Nothing to configure with no cart under the highlight, and a picker that wrote to
        // an empty stem would leave a line for a cart that is not there.
        let Some(cart) = self.shelf.current_cart() else {
            return;
        };
        // Nothing to choose for anything that is not a GBA cart: gpSP does not take Game Boy,
        // so mGBA is not a preference to record but the only answer there is. The picker is two
        // sockets to hop between, and opening it over one socket is a question that can only be
        // refused.
        if cart.system() != System::Gba {
            return;
        }
        let seat = slot_store::core_for(&root, &cart.stem);
        let now = self.now();
        let mut picker = CorePicker::open(seat, now);
        if self.core_faces_ready() {
            picker.start(now);
        }
        self.core_picker = Some(picker);
        self.shelf_mut().release_hold();
        self.play_held = None;
        self.fav_held = None;
        self.sys_held = None;
        self.mtp_held = None;
    }

    fn core_faces_ready(&self) -> bool {
        self.core_faces_stem
            .as_deref()
            .is_some_and(|stem| self.selected_stem() == Some(stem))
    }

    fn core_picker_input(&mut self, action: Action) {
        let press = match action {
            Action::GbaDown(Btn::Left) | Action::ShelfLeft => Press::Left,
            Action::GbaDown(Btn::Right) | Action::ShelfRight => Press::Right,
            Action::GbaDown(Btn::A) => Press::Keep,
            Action::GbaDown(Btn::B) => Press::Back,
            _ => return,
        };
        let now = self.now();
        let Some(picker) = &mut self.core_picker else {
            return;
        };
        let outcome = picker.press(press, now);
        if let Outcome::Write(core) = outcome {
            self.write_core(core);
        }
    }

    fn open_game_menu(&mut self) {
        if self.game_menu.is_some() {
            return;
        }
        if self.link_active() {
            return self.refuse();
        }
        if self.core == Core::Mgba {
            let wireless = self
                .seated()
                .is_some_and(|stem| self.link_mode(stem).0 == LinkKind::Wireless);
            if wireless {
                self.hud.toast(Toast::NeedsGpsp, self.now());
                return;
            }
            self.link_hardware = LinkKind::Cable;
            self.radio.ask(RadioJob::Warm);
            self.game_menu = Some(GameMenu::Pick(self.last_role));
            return;
        }
        if self.platform != Platform::Gba {
            self.hud.toast(Toast::NoLink, self.now());
            return;
        }
        let carried = self
            .seated()
            .and_then(|stem| self.auto_link(stem))
            .is_some_and(|(cart, _)| link_carried(&cart.code, &cart.title));
        if !carried {
            self.hud.toast(Toast::NoLink, self.now());
            return;
        }
        if self.core != Core::Gpsp {
            self.hud.toast(Toast::NeedsGpsp, self.now());
            return;
        }
        let hardware = self
            .seated()
            .map_or(LinkKind::Cable, |stem| self.link_mode(stem).0);
        self.link_hardware = hardware;
        self.radio.ask(RadioJob::Warm);
        self.game_menu = Some(GameMenu::Pick(self.last_role));
    }

    fn game_menu_shortcut(&mut self) {
        let Some(client_id) = self.link_client_id() else {
            return self.open_game_menu();
        };
        let now = self.now();
        self.game_menu = Some(GameMenu::Linked {
            role: LinkRow::from_client_id(client_id),
            worked: now,
            since: now,
            opened: true,
        });
    }

    fn end_link_from_menu(&mut self) {
        let role = self
            .link_client_id()
            .map_or(self.last_role, LinkRow::from_client_id);
        self.end_link();
        self.hud.toast(Toast::LinkEnded, self.now());
        self.unplug(role);
    }

    fn unplug(&mut self, role: LinkRow) {
        self.game_menu = Some(GameMenu::Unplug {
            role,
            since: self.now(),
        });
    }

    pub fn set_radio_jobs(&mut self, jobs: Box<dyn RadioJobs>) {
        self.radio = jobs;
    }

    fn game_menu_input(&mut self, action: Action) {
        let Some(menu) = self.game_menu else {
            return;
        };
        match menu {
            GameMenu::Pick(role) => match action {
                Action::GbaDown(Btn::Left) | Action::GbaDown(Btn::Right) => {
                    self.last_role = role.other();
                    self.game_menu = Some(GameMenu::Pick(role.other()));
                }
                Action::GbaDown(Btn::Select) => self.switch_hardware(),
                Action::GbaDown(Btn::A) => self.pick_link(role),
                Action::GbaDown(Btn::B) | Action::GameMenu => self.close_game_menu(),
                _ => {}
            },
            GameMenu::Working { .. } => {
                if action == Action::GbaDown(Btn::B) {
                    if let Some(starting) = &mut self.starting {
                        starting.starter.cancel();
                    }
                    if let Some(reload) = &mut self.reload {
                        reload.cancelled = true;
                    }
                }
            }
            GameMenu::Linked { opened: true, .. } => match action {
                Action::GbaDown(Btn::A) => self.end_link_from_menu(),
                Action::GbaDown(Btn::B) | Action::GameMenu => self.close_game_menu(),
                _ => {}
            },
            GameMenu::Linked { .. } => {}
            GameMenu::Failed { .. } => {
                if matches!(
                    action,
                    Action::GbaDown(Btn::A) | Action::GbaDown(Btn::B) | Action::GameMenu
                ) {
                    self.close_game_menu();
                }
            }
            GameMenu::Unplug { .. } => {}
        }
    }

    pub fn start_link(&mut self, starter: LinkStarter, client_id: u16) {
        self.start_link_from(starter, client_id, self.now());
    }

    fn start_link_from(&mut self, starter: LinkStarter, client_id: u16, since: Millis) {
        if let Some(mut old) = self.starting.replace(LinkStarting { starter, client_id }) {
            old.starter.cancel();
        }
        self.game_menu = Some(GameMenu::Working {
            role: LinkRow::from_client_id(client_id),
            step: LinkStep::Radio,
            since,
        });
    }

    fn switch_hardware(&mut self) {
        let Some(stem) = self.seated().map(str::to_string) else {
            return;
        };
        if !self.link_switchable(&stem) {
            return self.refuse();
        }
        let other = self.link_hardware.other();
        self.link_hardware = other;
        self.link_choices.insert(stem, other);
    }

    fn link_switchable(&self, stem: &str) -> bool {
        self.auto_link(stem).is_some_and(|(cart, auto)| {
            serial_option(self.link_hardware.other(), auto, &cart.code, &cart.title)
                != serial_option(self.link_hardware, auto, &cart.code, &cart.title)
        })
    }

    fn pick_link(&mut self, role: LinkRow) {
        let Some(stem) = self.seated().map(str::to_string) else {
            return;
        };
        let loaded = self.link_loaded.unwrap_or("auto");
        if self.core == Core::Mgba {
            let player = role.client_id() as u8;
            if self.link_player == Some(player) {
                return self.start_link(
                    LinkStarter::spawn(role.role(), link_port()),
                    role.client_id(),
                );
            }
            if self.snapshot.as_ref().is_some_and(|s| !s.resume_trusted()) {
                return self.refuse();
            }
            self.link_player = Some(player);
            self.link_reload = Some((stem.clone(), loaded));
            self.reload = Some(Reload {
                stem,
                role,
                cancelled: false,
                from: self.link_hardware,
                from_serial: loaded,
                fallback: false,
            });
            self.game_menu = Some(GameMenu::Working {
                role,
                step: LinkStep::Radio,
                since: self.now(),
            });
            return;
        }
        let (_, serial) = self.link_mode(&stem);
        if serial == loaded {
            return self.start_link(
                LinkStarter::spawn(role.role(), link_port()),
                role.client_id(),
            );
        }
        if self.snapshot.as_ref().is_some_and(|s| !s.resume_trusted()) {
            return self.refuse();
        }
        self.link_reload = Some((stem.clone(), serial));
        self.reload = Some(Reload {
            stem,
            role,
            cancelled: false,
            from: self.link_hardware.other(),
            from_serial: loaded,
            fallback: false,
        });
        self.game_menu = Some(GameMenu::Working {
            role,
            step: LinkStep::Radio,
            since: self.now(),
        });
    }

    fn refuse_seated(&mut self) {
        self.close_game_menu();
        self.pending = None;
        let caught = self.seat();
        let cart = match &mut self.phase {
            Phase::Playing { cart } => Some(std::mem::take(cart)),
            Phase::Doze { cart } => {
                *cart = None;
                None
            }
            _ => None,
        };
        if let Some(cart) = cart {
            self.refuse_out(cart, caught);
        }
    }

    fn close_game_menu(&mut self) {
        self.game_menu = None;
        if let Some(mut starting) = self.starting.take() {
            starting.starter.cancel();
        }
        if !self.link_active() {
            self.radio.ask(RadioJob::Cool);
        }
        let uncollected =
            self.link_reload.is_some() && self.reload.as_ref().is_some_and(|r| !r.fallback);
        if uncollected {
            self.link_reload = None;
            self.reload = None;
        } else if let Some(reload) = &mut self.reload {
            reload.cancelled = true;
        }
    }

    fn working_role(&self, client_id: u16) -> (LinkRow, Millis) {
        match self.game_menu {
            Some(GameMenu::Working { role, since, .. }) => (role, since),
            _ => (LinkRow::from_client_id(client_id), self.now()),
        }
    }

    fn poll_link(&mut self) {
        if self.power_menu.is_some() {
            return;
        }
        let Some(mut starting) = self.starting.take() else {
            return;
        };
        match starting.starter.poll() {
            None => self.starting = Some(starting),
            Some(LinkProgress::At(step)) => {
                if let Some(GameMenu::Working { role, since, .. }) = self.game_menu {
                    self.game_menu = Some(GameMenu::Working { role, step, since });
                }
                self.starting = Some(starting);
            }
            Some(LinkProgress::Ready(link)) => {
                let (role, worked) = self.working_role(starting.client_id);
                self.game_menu = Some(GameMenu::Linked {
                    role,
                    worked,
                    since: self.now(),
                    opened: false,
                });
                self.begin_link(starting.client_id);
                self.link_transport = Some((starting.client_id, Box::new(link)));
            }
            Some(LinkProgress::Failed(LinkFail::Cancelled)) => self.game_menu = None,
            Some(LinkProgress::Failed(fail)) => {
                let (role, worked) = self.working_role(starting.client_id);
                self.game_menu = Some(GameMenu::Failed {
                    role,
                    fail,
                    worked,
                    since: self.now(),
                });
            }
        }
    }

    fn write_core(&self, core: Core) {
        let (Some(root), Some(cart)) = (self.root.clone(), self.shelf.current_cart()) else {
            return;
        };
        if let Err(e) = slot_store::write_selected_core(&root, &cart.stem, core) {
            eprintln!("slot: core: could not write selected_core.txt: {e}");
        }
    }

    fn begin_power_off(&mut self) {
        if self.powering_off {
            return;
        }
        if self.link_active() {
            self.end_link();
        }
        self.close_game_menu();
        self.close_cheat_menu();
        self.close_palette_browser();
        self.powering_off = true;
        self.act_at = self.now() + SHUTDOWN_SHOW_MS;
        self.set_led(LedState::Off);
    }

    pub fn on_battery(&mut self, b: Battery) {
        if b.percent > BATTERY_CRITICAL || self.powering_off {
            return;
        }
        if matches!(b.charge, Charge::Charging | Charge::Full) {
            return;
        }
        self.flush_resume();
        self.begin_power_off();
    }

    fn doze_expired(&self) -> bool {
        if self.link_active() {
            return false;
        }
        let (Phase::Doze { .. }, Some(power)) = (&self.phase, &self.power) else {
            return false;
        };
        self.now().saturating_sub(self.dozed_at) >= power.timeout().as_millis() as Millis
    }

    pub fn settle_saves(&mut self) {
        self.state_writer.flush();
        if let Some(h) = self.pending_save.take() {
            let _ = h.join();
        }
    }

    fn autosave(&mut self) {
        self.autosave_at = self.now() + AUTOSAVE_MS;
        self.settle_saves();
        let (Some(root), Some(snapshot), Some(cart)) = (&self.root, &self.snapshot, self.seated())
        else {
            return;
        };
        let Some(state) = snapshot.state() else {
            eprintln!("slot: autosave: the core gave up no state");
            return;
        };
        let Some(system) = self.system_of(cart) else {
            eprintln!("slot: autosave: no cart in the shelf for {cart}");
            return;
        };
        let (state, sav) = trusted_write(snapshot.as_ref(), state, "autosave");
        let (root, platform, core, cart, system) = (
            root.clone(),
            self.platform,
            self.core,
            cart.to_owned(),
            system,
        );
        let failing = self.card_failing.clone();
        let write = move || {
            let written = persist::flush(
                &root,
                platform,
                core,
                system,
                &cart,
                state.as_deref(),
                sav.as_deref(),
            );
            if let Err(e) = &written {
                eprintln!("slot: autosave: {e}");
            }
            failing.store(written.is_err(), Ordering::Relaxed);
        };
        match std::thread::Builder::new()
            .name("slot-autosave".into())
            .spawn(write)
        {
            Ok(h) => self.pending_save = Some(h),
            Err(e) => eprintln!("slot: autosave: no writer thread: {e}"),
        }
    }

    pub fn flush_resume(&mut self) {
        self.settle_saves();
        self.autosave_at = self.now() + AUTOSAVE_MS;
        let (Some(root), Some(snapshot), Some(cart)) = (&self.root, &self.snapshot, self.seated())
        else {
            return;
        };
        let Some(state) = snapshot.state() else {
            eprintln!("slot: flush: the core gave up no state");
            return;
        };
        let Some(system) = self.system_of(cart) else {
            eprintln!("slot: flush: no cart in the shelf for {cart}");
            return;
        };
        let (state, sav) = trusted_write(snapshot.as_ref(), state, "flush");
        let written = persist::flush(
            root,
            self.platform,
            self.core,
            system,
            cart,
            state.as_deref(),
            sav.as_deref(),
        );
        if let Err(e) = &written {
            eprintln!("slot: flush: {e}");
        }
        self.card_failing.store(written.is_err(), Ordering::Relaxed);
    }

    /// The ring for the cart in the slot. `None` outside the binary, where there is no
    /// content root, which reads as a cart that has never been saved.
    /// The machine of the cart that owns `stem` — the folder its save and its states live under.
    /// Read from the shelf rather than kept on `App`, because the shelf already carries every
    /// cart's machine and a second copy is exactly the thing that drifts.
    fn system_of(&self, stem: &str) -> Option<System> {
        self.shelf
            .carts
            .iter()
            .find(|c| c.stem == stem)
            .map(|c| c.system())
    }

    fn ring(&self) -> Option<StateRing> {
        let (Some(root), Some(cart)) = (&self.root, self.seated()) else {
            return None;
        };
        self.system_of(cart)?;
        Some(StateRing::new(root, self.platform, self.core, cart))
    }

    fn seated(&self) -> Option<&str> {
        match &self.phase {
            Phase::Playing { cart } | Phase::Polaroids { cart } => Some(cart),
            _ => None,
        }
    }

    fn entries(&self) -> Vec<StateEntry> {
        self.ring().and_then(|r| r.list().ok()).unwrap_or_default()
    }

    fn open_polaroids(&mut self) {
        if self.link_active() {
            return self.refuse();
        }
        let entries = self.entries();
        if entries.is_empty() {
            return self.refuse();
        }
        let Phase::Playing { cart } = &mut self.phase else {
            return;
        };
        let cart = std::mem::take(cart);
        let mut p = Polaroids::new(entries);
        p.set_undo(self.undo_label());
        self.polaroids = Some(p);
        self.phase = Phase::Polaroids { cart };
        self.push_hint_faces();
    }

    fn close_polaroids(&mut self) {
        let Phase::Polaroids { cart } = &mut self.phase else {
            return;
        };
        let cart = std::mem::take(cart);
        self.polaroids = None;
        self.phase = Phase::Playing { cart };
    }

    fn load_selected(&mut self) {
        let state = self
            .polaroids
            .as_ref()
            .and_then(|p| p.selected())
            .map(|e| e.state.clone());
        let refused = state.map(|state| self.load_file(&state)) == Some(false);
        if !refused {
            self.close_polaroids();
        }
    }

    fn delete_selected(&mut self) {
        let stamp = self
            .polaroids
            .as_ref()
            .and_then(|p| p.selected())
            .map(|e| e.stamp.clone());
        let (Some(stamp), Some(ring)) = (stamp, self.ring()) else {
            return;
        };
        if let Err(e) = ring.remove(&stamp) {
            eprintln!("slot: delete: {e}");
            return;
        }
        if self.undo_targets(&stamp) {
            self.pending = None;
        }
        let Some(p) = &mut self.polaroids else {
            return;
        };
        p.remove_selected();
        if p.is_empty() {
            self.close_polaroids();
        }
    }

    fn undo_targets(&self, stamp: &str) -> bool {
        match self.pending.as_ref() {
            Some((PendingUndo::Save { stamp: pending, .. }, _)) => pending == stamp,
            _ => false,
        }
    }

    fn load_newest(&mut self) {
        let Some(newest) = self.entries().first().map(|e| e.state.clone()) else {
            return self.refuse();
        };
        self.load_file(&newest);
    }

    fn load_file(&mut self, state: &Path) -> bool {
        if !self.may_load_state() {
            self.refuse();
            return false;
        }
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        let bytes = match std::fs::read(state) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("slot: load: {e}");
                return false;
            }
        };
        let prior = snapshot.state();
        snapshot.load(bytes);
        self.hud.toast(Toast::StateLoaded, self.now());
        if let Some(prior) = prior {
            self.pending = Some((PendingUndo::Load { prior }, self.now()));
        }
        true
    }

    /// `SELECT+A` and nothing else reaches here. A state with no picture is still worth
    /// keeping: the switcher draws a blank card rather than losing the save.
    ///
    /// Declines outright when the live core refused the resume it was opened with — the same
    /// condition `trusted_write` withholds from `flush`/`eject` for. This is the one durable
    /// sink that guard does not reach, because it is not a write-back over an existing file:
    /// `ring.push` is a deliberate ring buffer, and once it holds `RING_MAX` entries, pushing
    /// an eleventh evicts the oldest to make room. A core running on its own default machine
    /// has nothing worth keeping in that slot, so pushing it would not just waste an entry —
    /// it would delete a real one to make room for a placeholder. Refused the same way every
    /// other "nothing to do here" action in this file is, via `refuse()`: the player gets the
    /// same shake `load_newest`/`open_polaroids` already answer with, rather than a save that
    /// silently did not happen.
    fn save_state(&mut self) {
        let (Some(ring), Some(snapshot)) = (self.ring(), &self.snapshot) else {
            return;
        };
        if !snapshot.resume_trusted() {
            eprintln!("slot: save: the core refused the resume it was given, not pushing a state");
            return self.refuse();
        }
        let Some(state) = snapshot.state() else {
            eprintln!("slot: save: the core gave up no state");
            return;
        };
        let thumb = snapshot.thumb().unwrap_or_default();
        let stamp = free_stamp(&ring, self.wall_secs());
        let evicted = doomed(&ring);
        if let Err(e) = ring.push(&state, &thumb, &stamp) {
            eprintln!("slot: save: {e}");
            return;
        }
        self.hud.toast(Toast::StateSaved, self.now());
        self.pending = Some((PendingUndo::Save { stamp, evicted }, self.now()));
    }

    pub fn undo_available(&self, now: Millis) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_sub(*at) <= UNDO_GRACE_MS)
    }

    pub fn undo_label(&self) -> Option<&'static str> {
        if !self.undo_available(self.now()) {
            return None;
        }
        match self.pending.as_ref()?.0 {
            PendingUndo::Save { .. } => Some(lang::UNDO_SAVE),
            PendingUndo::Load { .. } => Some(lang::UNDO_LOAD),
        }
    }

    pub fn set_legend_faces(&mut self, faces: Vec<TexId>) {
        self.legend_faces = faces;
        self.push_hint_faces();
    }

    pub fn set_undo_face(&mut self, face: Option<TexId>) {
        self.undo_face = face;
        self.push_hint_faces();
    }

    fn push_hint_faces(&mut self) {
        let mut faces = self.legend_faces.clone();
        faces.extend(self.undo_face);
        if let Some(p) = &mut self.polaroids {
            p.set_hint_faces(faces);
        }
    }

    /// The mark a favourite wears, and the corner indicator on the ordinary shelf and in the
    /// favourites, each with the size it was rasterised at. Uploaded at boot: none of them changes.
    pub fn set_fav_faces(
        &mut self,
        star: (TexId, u32, u32),
        ind_off: (TexId, u32, u32),
        ind_on: (TexId, u32, u32),
    ) {
        self.fav_star = Some(star);
        self.fav_ind = Some((ind_off, ind_on));
    }

    /// The HUD glyphs, in `Icon::ALL` order. Uploaded once: they never change.
    pub fn set_icon_faces(&mut self, faces: Vec<TexId>) {
        self.hud.set_icons(faces);
    }

    pub fn set_toast_faces(&mut self, faces: Vec<TexId>) {
        self.hud.set_toasts(faces);
    }

    pub fn toast(&self) -> Option<Toast> {
        self.hud.said(self.now())
    }

    /// Show a toast from outside `App` — the in-game cheat toggle, which the session drives
    /// because the core it reaches lives on the emulator thread. `now` is read off the app's own
    /// clock so the fade lines up with everything else the HUD says.
    pub fn toast_now(&mut self, t: Toast) {
        self.hud.toast(t, self.now());
    }

    pub fn card_alarm(&self) -> bool {
        self.hud.alarmed()
    }

    /// One shot, and it hands the game back the way loading does. Undoing an undo would be a
    /// redo, and the switcher is not a place to sit and shuffle.
    pub fn undo(&mut self, now: Millis) {
        if !self.undo_available(now) {
            self.pending = None;
            return;
        }
        if matches!(&self.pending, Some((PendingUndo::Load { .. }, _))) && !self.may_load_state() {
            return self.refuse();
        }
        let Some((what, _)) = self.pending.take() else {
            return;
        };
        match what {
            PendingUndo::Save { stamp, evicted } => self.undo_save(&stamp, evicted),
            PendingUndo::Load { prior } => {
                if let Some(snapshot) = &self.snapshot {
                    snapshot.load(prior);
                }
            }
        }
        self.close_polaroids();
    }

    fn undo_save(&self, stamp: &str, evicted: Option<(String, Vec<u8>, Vec<u8>)>) {
        let Some(ring) = self.ring() else {
            return;
        };
        if let Err(e) = ring.remove(stamp) {
            eprintln!("slot: undo: {e}");
            return;
        }
        let Some((stamp, state, thumb)) = evicted else {
            return;
        };
        if let Err(e) = ring.push(&state, &thumb, &stamp) {
            eprintln!("slot: undo: {e}");
        }
    }

    pub fn polaroid_entries(&self) -> &[StateEntry] {
        match &self.polaroids {
            Some(p) => &p.entries,
            None => &[],
        }
    }

    pub fn set_polaroid_faces(&mut self, faces: Vec<TexId>) {
        if let Some(p) = &mut self.polaroids {
            p.set_faces(faces);
        }
    }

    pub fn polaroid_stamp(&self) -> Option<&str> {
        self.polaroids
            .as_ref()
            .and_then(|p| p.selected())
            .map(|e| e.stamp.as_str())
    }

    pub fn polaroid_title(&self, now: &str) -> String {
        self.polaroids
            .as_ref()
            .map_or_else(String::new, |p| p.title(now))
    }

    pub fn set_polaroid_title_face(&mut self, face: TexId) {
        if let Some(p) = &mut self.polaroids {
            p.set_title_face(Some(face));
        }
    }
}

fn trusted_write(
    snapshot: &dyn Snapshot,
    state: Vec<u8>,
    verb: &str,
) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let state = if snapshot.resume_trusted() {
        Some(state)
    } else {
        eprintln!(
            "slot: {verb}: the core refused the resume it was given, not overwriting the saved one"
        );
        None
    };
    let sav = snapshot.save_ram();
    let sav = if snapshot.save_ram_trusted() {
        sav
    } else {
        if sav.is_some() {
            eprintln!(
                "slot: {verb}: the core refused the save ram it was given, not overwriting the saved one"
            );
        }
        None
    };
    (state, sav)
}

fn up(level: u8, step: u8, max: u8) -> u8 {
    level.saturating_add(step).min(max)
}

fn shader_value(shader: Shader) -> QuickValue {
    match shader {
        Shader::Off => QuickValue::Off,
        Shader::Lcd3x => QuickValue::Lcd3x,
        Shader::Grid => QuickValue::Grid,
        Shader::Dot => QuickValue::Dot,
        Shader::Simpletex => QuickValue::Simpletex,
    }
}

fn ff_next(from: u8, right: bool) -> u8 {
    let n = FF_SPEEDS.len();
    let at = FF_SPEEDS.iter().position(|&v| v == from).unwrap_or(0);
    FF_SPEEDS[if right {
        (at + 1) % n
    } else {
        (at + n - 1) % n
    }]
}

fn clock_screen(utc: i64, offset_min: i16, from_menu: bool) -> Phase {
    Phase::SetClock {
        picker: ClockPicker::local(utc, offset_min),
        seed: utc - utc.rem_euclid(60),
        from_menu,
    }
}

fn system_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn doomed(ring: &StateRing) -> Option<(String, Vec<u8>, Vec<u8>)> {
    let entries = ring.list().ok()?;
    let oldest = entries.get(RING_MAX - 1)?;
    let (state, thumb) = ring.read(&oldest.stamp).ok()?;
    Some((oldest.stamp.clone(), state, thumb))
}

fn free_stamp(ring: &StateRing, now: i64) -> String {
    let taken: Vec<String> = ring
        .list()
        .map(|l| l.into_iter().map(|e| e.stamp).collect())
        .unwrap_or_default();
    let mut secs = now;
    let mut stamp = format_stamp(secs);
    while taken.contains(&stamp) {
        secs += 1;
        stamp = format_stamp(secs);
    }
    stamp
}

fn centred_top(rows: usize) -> f32 {
    (OUT_H as f32 - POWER_MENU_PITCH * rows as f32) / 2.0
}

fn draw_menu_rows(
    faces: &[(TexId, u32, u32)],
    index: Option<usize>,
    top: f32,
    out: &mut Vec<Draw>,
) {
    for (row, (tex, w, h)) in faces.iter().copied().enumerate() {
        let y = top + POWER_MENU_PITCH * row as f32;
        let x = ((OUT_W as f32 - w as f32) / 2.0).round();
        if index == Some(row) {
            out.push(Draw::Rect {
                x,
                y: y + POWER_MENU_BAR_INSET,
                w: w as f32,
                h: POWER_MENU_PITCH - 2.0 * POWER_MENU_BAR_INSET,
                colour: slot_ui::edge(),
            });
        }
        out.push(Draw::Tex {
            x,
            y: y + (POWER_MENU_PITCH - h as f32) / 2.0,
            w: w as f32,
            h: h as f32,
            tex,
            alpha: 1.0,
        });
    }
}
