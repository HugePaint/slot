use slot_store::gb::Class;
use slot_store::{clean_label, Cart, Outline, Platform, System};

use crate::art;
use crate::shell::{shell_for, Finish, Shell, DEFAULT_SHELL};
use crate::silhouette::{
    cart_depth, cart_mask, detail_mask, gb_cart_mask, GbShell,
};
use crate::text;

/// Where the time inside `cart_face` goes, accumulated over every call since the last take.
///
/// A cart face costs something like 70 ms on this hardware, and the device has no profiler
/// and no console: this is how "which pass is actually expensive" gets answered from a
/// `boot.log` on a card instead of by argument.
pub mod face_profile {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    pub const PHASES: [&str; 6] = ["shell", "label", "mould", "recess", "paste", "clip"];

    static NANOS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

    pub fn add(phase: usize, d: Duration) {
        if let Some(slot) = NANOS.get(phase) {
            slot.fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
        }
    }

    /// Milliseconds per phase, and resets the counters.
    pub fn take_ms() -> [f64; 6] {
        let mut out = [0.0; 6];
        for (i, slot) in NANOS.iter().enumerate() {
            out[i] = slot.swap(0, Ordering::Relaxed) as f64 / 1e6;
        }
        out
    }

    /// `shell 12.3 label 4.5 ...` — one line, for the boot log.
    pub fn line() -> String {
        let ms = take_ms();
        PHASES
            .iter()
            .zip(ms)
            .map(|(name, v)| format!("{name} {v:.1}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The traced outline's own aspect, so `cart.svg` rasterises unstretched. Three across a
/// 720 wide row exactly, so the shelf can show a neighbour either side of the selection.
pub const CART_W: u32 = 240;
pub const CART_H: u32 = 135;
/// Internal face resolution multiplier. The cart face (shell, label, masks) is rasterised and
/// composited at `FACE_SCALE` times the logical cart size, then the shelf draws it into the
/// logical `CART_W`×`CART_H` quad. This keeps the centre cart crisp when the shelf scales it up
/// to 1.5x (and any larger scale): the texture is only ever downsampled, never stretched.
pub const FACE_SCALE: u32 = 2;
pub const FACE_W: u32 = CART_W * FACE_SCALE;
pub const FACE_H: u32 = CART_H * FACE_SCALE;

/// Seated size of a cart in the bay, in the units the slot chrome measures the mouth against.
pub const SEATED_W: u32 = 240;
const SEATED_H: u32 = 141;
const GB_SEATED_H: u32 = 259;

/// Upstream Game Boy cart geometry, kept beside the shelf's own `CartSize`. `gb_cart_shadow`
/// rasterises at these; the row itself draws `size_for`.
pub const GB_CART_W: u32 = 312;
pub const GB_CART_H: u32 = (GB_CART_W * GB_SEATED_H + SEATED_W / 2) / SEATED_W;

/// A cart's own size, per machine — and the row it stands on.
///
/// The Advance cart is landscape and its shelf stands three across; a Game Boy or Game Boy
/// Color cart is portrait, so it gets a row of its own, drawn at its own size rather than
/// enlarged. A portrait cart at the Advance row's 1.5x would be 414 px tall on a 480 px panel,
/// which is a cart with a shelf behind it rather than the other way round. Three 240 wide carts
/// fill the 720 panel edge to edge, so the pitch is exactly the cart's width and the neighbours
/// sit one cart apart.
///
/// The label panel is per mille of the face, x0 y0 x1 y1. The Advance cart's is the reference's;
/// the Game Boy's is measured (42 x 37 mm on a 57 x 65.5 mm shell — 7.5 mm in from each side,
/// 6 mm down), and nearly square where the Advance cart's is wide.
#[derive(Copy, Clone)]
pub struct CartSize {
    pub w: u32,
    pub h: u32,
    pub pitch: f32,
    /// How much bigger the selected cart is than its neighbours.
    pub middle: f32,
    /// And how big the neighbours are, as a share of their own size.
    pub side: f32,
    label: [u32; 4],
}

pub const fn size_for(system: System) -> CartSize {
    match system {
        System::Gba => CartSize {
            w: CART_W,
            h: CART_H,
            pitch: 360.0,
            middle: 1.5,
            side: 1.0,
            label: [90, 228, 910, 863],
        },
        System::Gb | System::Gbc => CartSize {
            w: 240,
            h: 276,
            pitch: 240.0,
            middle: 1.0,
            side: 0.7,
            label: [132, 92, 868, 657],
        },
    }
}

impl CartSize {
    pub const fn face_w(&self) -> u32 {
        self.w * FACE_SCALE
    }

    pub const fn face_h(&self) -> u32 {
        self.h * FACE_SCALE
    }

    /// Where the row's feet stand: the carts stand on the row rather than float on it.
    ///
    /// A portrait cart is lifted clear of the title. The formula is right for the Advance row,
    /// where the gap it leaves under a 135 px cart is 37 px and the title has room in it. A
    /// 276 px cart pushes its foot 70 px further down, and the same gap collapses to 2 px with
    /// the title jammed against the row. Lifting the portrait row restores it — and takes the
    /// title with it, since `shelf_title_y` is derived from the foot: the row comes up by the
    /// whole lift and the title by half of it, so the gap grows by the other half.
    pub fn foot_y(&self) -> f32 {
        let derived = (crate::draw::OUT_H + self.h) as f32 / 2.0;
        if self.h == CART_H {
            derived
        } else {
            derived - 32.0
        }
    }

    /// The paper label's panel on the face, in face pixels.
    pub fn label_panel(&self) -> (u32, u32, u32, u32) {
        self.label_panel_at(self.face_w(), self.face_h())
    }

    /// The same panel against any size, for a caller drawing the cart smaller than its face —
    /// the row draws placeholders at the quad's size and needs the panel in those units.
    ///
    /// **Corners, not a size**: `(x0, y0, x1, y1)`, exactly as `label` stores them. A caller
    /// that wants to fill or paste *inside* the panel wants `label_box` instead; the two were
    /// confused once and cost every card with a game on it its boot.
    pub fn label_panel_at(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let p = |v: u32, of: u32| (of * v + 500) / 1000;
        (
            p(self.label[0], w),
            p(self.label[1], h),
            p(self.label[2], w),
            p(self.label[3], h),
        )
    }

    /// The panel as an origin and a size — `(x, y, w, h)`. What every caller that draws into
    /// the panel wants, because a pasted label is sized, not cornered: `label_panel`'s third
    /// and fourth numbers are the far edge, and handing those to `paste_label` as a width and
    /// a height pastes a label wider and taller than the cart it is going onto.
    pub fn label_box(&self) -> (u32, u32, u32, u32) {
        let (x0, y0, x1, y1) = self.label_panel();
        (x0, y0, x1 - x0, y1 - y0)
    }
}

pub const fn label_panel(w: u32, h: u32) -> (u32, u32, u32, u32) {
    (
        (w * 135 + 500) / 1000,
        (h * 231 + 500) / 1000,
        (w * 875 + 500) / 1000,
        (h * 882 + 500) / 1000,
    )
}

/// The label panel is laid out at the higher face resolution, so the title text is rasterised
/// sharp and only ever downsampled when drawn at the logical cart size.
pub const LABEL_X: u32 = label_panel(FACE_W, FACE_H).0;
pub const LABEL_Y: u32 = label_panel(FACE_W, FACE_H).1;
pub const LABEL_W: u32 = label_panel(FACE_W, FACE_H).2 - LABEL_X;
pub const LABEL_H: u32 = label_panel(FACE_W, FACE_H).3 - LABEL_Y;

pub const fn gb_label_panel(w: u32, h: u32) -> (u32, u32, u32, u32) {
    (
        (w * 147 + 500) / 1000,
        (h * 291 + 500) / 1000,
        (w * 854 + 500) / 1000,
        (h * 868 + 500) / 1000,
    )
}

pub const GB_LABEL_X: u32 = gb_label_panel(GB_CART_W, GB_CART_H).0;
pub const GB_LABEL_Y: u32 = gb_label_panel(GB_CART_W, GB_CART_H).1;
pub const GB_LABEL_W: u32 = gb_label_panel(GB_CART_W, GB_CART_H).2 - GB_LABEL_X;
pub const GB_LABEL_H: u32 = gb_label_panel(GB_CART_W, GB_CART_H).3 - GB_LABEL_Y;

const PAD: u32 = 10 * FACE_SCALE;
const MAX_LINES: usize = 3;
const MAX_PX: f32 = LABEL_H as f32 / (MAX_LINES as f32 * 1.36);
const MIN_PX: f32 = 10.0 * FACE_SCALE as f32;

/// How far the translucent edge reaches in. Zero at this depth exactly, so a pixel any
/// further in is the plastic's own colour.
const RIM: u32 = 4 * FACE_SCALE;

pub struct CartFace {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

pub fn gb_shell_of(cart: &Cart) -> Option<GbShell> {
    match cart.platform {
        Platform::Gba => None,
        Platform::Gb | Platform::Gbc => Some(match cart.shell.map(|c| c.outline) {
            Some(Outline::Notched) => GbShell::Notched,
            Some(Outline::Rounded) => GbShell::Rounded,
            _ => match slot_store::gb::class(&cart.rom) {
                Class::ColourOnly => GbShell::Rounded,
                Class::Original | Class::DualMode => GbShell::Notched,
            },
        }),
    }
}

pub fn seated_box(platform: Platform) -> (u32, u32) {
    match platform {
        Platform::Gba => (SEATED_W, SEATED_H),
        Platform::Gb | Platform::Gbc => (SEATED_W, GB_SEATED_H),
    }
}

pub fn cart_box(platform: Platform) -> (u32, u32) {
    let system = match platform {
        Platform::Gba => System::Gba,
        Platform::Gb => System::Gb,
        Platform::Gbc => System::Gbc,
    };
    let size = size_for(system);
    (size.w, size.h)
}

/// The cart's own shape in black. Drawn under a side cart so the dimming is a cart in shadow
/// rather than a cart you can see through: over a wallpaper a translucent face is a ghost,
/// and the shelf's carts are solid objects.
pub fn cart_shadow(size: CartSize, system: System) -> CartFace {
    let (w, h) = (size.face_w(), size.face_h());
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for cover in cart_mask(system, w, h) {
        rgba.extend_from_slice(&[0, 0, 0, *cover]);
    }
    CartFace { rgba, w, h }
}

pub fn gb_cart_shadow(shell: GbShell) -> CartFace {
    let mask = gb_cart_mask(shell);
    let (w, h) = (GB_CART_W, GB_CART_H);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for cover in mask {
        rgba.extend_from_slice(&[0, 0, 0, *cover]);
    }
    CartFace { rgba, w, h }
}

/// A cart with nothing on it: the default shell, its moulding, and the empty recess the label
/// would sit in — no label.
///
/// The stand-in for a cart whose own face has not been built yet. A jump across the alphabet
/// crosses hundreds of carts and they cannot all be rasterised, but they can all be *a cart*:
/// this one. Drawn where that cart is, it slides past as a cartridge rather than as the label's
/// colour alone, which read as a bar of paint going by.
pub fn cart_placeholder(size: CartSize, system: System) -> CartFace {
    // The stand-in wears the current theme's shell rather than the table's dark grey, which is
    // what makes it belong to the light theme's furniture too.
    let shell = themed_shell(&DEFAULT_SHELL);
    let mut face = shell_face(&shell, size, system);
    mould_detail(&mut face, &shell, size, system);
    recess_label(&mut face, &shell, size.label_box());
    clip_to_silhouette(&mut face, size, system);
    face
}

/// The built-in shell as the current theme wants it. On the dark theme that is the table's own
/// colour; on the light theme a built-in shell takes the housing's — the off-white the top and
/// bottom bands are drawn in — so a cart reads as part of the same device as the furniture
/// around it. `card_face` states the same rule for a card's own Advance art; this is the
/// built-in path's half of it, and the reason the blank cart has to be re-minted when the
/// theme changes and not only when the machine does.
///
/// Note this reads the mode at *bake* time, so it is a colour that goes stale the moment
/// `palette::set_mode` runs: every caller of `cart_placeholder` / `cart_face` is expected to
/// re-bake after a theme change.
fn themed_shell(shell: &Shell) -> Shell {
    match crate::palette::mode() {
        crate::palette::Mode::Light => {
            let h = crate::slot_chrome::housing();
            Shell {
                colour: [
                    (h[0] * 255.0) as u8,
                    (h[1] * 255.0) as u8,
                    (h[2] * 255.0) as u8,
                ],
                finish: shell.finish,
            }
        }
        crate::palette::Mode::Dark => *shell,
    }
}

pub fn cart_face(cart: &Cart) -> CartFace {
    let shell = shell_for(cart);
    let size = size_for(cart.system());
    // A card that supplies its own cart art is drawn from that instead of from the built-in
    // silhouette: see `cart_art`. Nothing below changes for a card that does not.
    if let Some(face) = card_face(cart, &shell, size) {
        return face;
    }
    let (_, _, lw, lh) = size.label_box();
    let art = cart.label.as_deref().and_then(|p| art::cover(p, lw, lh));
    cart_face_with(cart, art)
}

pub fn label_size(cart: &Cart) -> (u32, u32) {
    let (_, _, lw, lh) = size_for(cart.system()).label_box();
    (lw, lh)
}

pub fn cart_face_with(cart: &Cart, art: Option<Vec<u8>>) -> CartFace {
    let shell = themed_shell(&shell_for(cart));
    let system = cart.system();
    let size = size_for(system);
    let (lx, ly, lw, lh) = size.label_box();

    let t = std::time::Instant::now();
    let mut face = shell_face(&shell, size, system);
    face_profile::add(0, t.elapsed());

    let t = std::time::Instant::now();
    let label = match art {
        Some(rgba) => rgba,
        None => sized_label(&label_text(cart), lw, lh),
    };
    face_profile::add(1, t.elapsed());

    let t = std::time::Instant::now();
    mould_detail(&mut face, &shell, size, system);
    face_profile::add(2, t.elapsed());

    let t = std::time::Instant::now();
    recess_label(&mut face, &shell, (lx, ly, lw, lh));
    face_profile::add(3, t.elapsed());

    let t = std::time::Instant::now();
    paste_label(&mut face, &label, (lx, ly, lw, lh));
    face_profile::add(4, t.elapsed());

    let t = std::time::Instant::now();
    clip_to_silhouette(&mut face, size, system);
    face_profile::add(5, t.elapsed());

    face
}

/// Colour is left alone and only alpha is cut, because the sprite pass blends straight
/// alpha rather than premultiplied.
fn clip_to_silhouette(face: &mut CartFace, size: CartSize, system: System) {
    for (px, cover) in face
        .rgba
        .chunks_exact_mut(4)
        .zip(cart_mask(system, size.face_w(), size.face_h()))
    {
        px[3] = ((px[3] as u32 * *cover as u32 + 127) / 255) as u8;
    }
}

pub fn label_colour(title: &str) -> [u8; 3] {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in title.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hsv_to_rgb((h % 360) as f32, 0.52, 0.74)
}

pub fn label_text(cart: &Cart) -> String {
    clean_label(&cart.stem)
}

fn shell_face(shell: &Shell, size: CartSize, system: System) -> CartFace {
    let (w, h) = (size.face_w(), size.face_h());
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let edge = rim_colour(shell.colour);
    for (i, depth) in cart_depth(system, w, h).iter().enumerate() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let c = match shell.finish {
            Finish::Solid => shell.colour,
            Finish::Translucent | Finish::Glitter => {
                let body = through(shell.colour, inside(system, w, h, x, y).unwrap_or(shell.colour));
                let c = lerp(edge, body, (*depth as u32).min(RIM), RIM);
                if shell.finish == Finish::Glitter && fleck(x, y) {
                    lerp(c, [0xff; 3], 100, 255)
                } else {
                    c
                }
            }
        };
        rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
    }
    CartFace { rgba, w, h }
}

const SURFACE: u32 = 150;

const BOARD: [u8; 3] = [0x2c, 0x96, 0x52];
const TRACE: [u8; 3] = [0x5a, 0xb4, 0x74];
const GOLD: [u8; 3] = [0xe6, 0xb4, 0x46];

fn through(plastic: [u8; 3], behind: [u8; 3]) -> [u8; 3] {
    let filter =
        |a: [u8; 3], b: [u8; 3]| [0, 1, 2].map(|c| (a[c] as u32 * b[c] as u32 / 255) as u8);
    lerp(
        filter(plastic, behind),
        filter(plastic, plastic),
        SURFACE,
        255,
    )
}

fn inside(system: System, w: u32, h: u32, x: u32, y: u32) -> Option<[u8; 3]> {
    let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
    let (x0, x1, top, c0, c1, contacts_from, traces_from) = match system {
        System::Gba => (0.135, 0.875, 0.231, 0.17, 0.84, 0.85, 0.76),
        System::Gb | System::Gbc => (0.075, 0.925, 0.07, 0.1, 0.9, 0.905, 0.84),
    };
    if !(x0..x1).contains(&fx) || fy < top {
        return None;
    }
    let pitch = (c1 - c0) / 32.0;
    let on_contact = (c0..c1).contains(&fx) && ((fx - c0) % pitch) < pitch * 0.6;
    if on_contact && fy >= contacts_from {
        return Some(GOLD);
    }
    if on_contact && ((fx - c0) % pitch) < pitch * 0.2 && fy >= traces_from {
        Some(TRACE)
    } else {
        Some(BOARD)
    }
}

fn fleck(x: u32, y: u32) -> bool {
    let mut h = x.wrapping_mul(0x9e37_79b9) ^ y.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h.is_multiple_of(100)
}

fn rim_colour(base: [u8; 3]) -> [u8; 3] {
    let mean = (base[0] as u16 + base[1] as u16 + base[2] as u16) / 3;
    base.map(|c| {
        let grey = (3 * c as u16 + mean) / 4;
        (grey + (255 - grey) * 2 / 5) as u8
    })
}

fn lerp(a: [u8; 3], b: [u8; 3], num: u32, den: u32) -> [u8; 3] {
    let mut out = [0u8; 3];
    for c in 0..3 {
        out[c] = ((a[c] as u32 * (den - num) + b[c] as u32 * num) / den) as u8;
    }
    out
}

/// The wall of the moulded recess the label sits in. Light comes from the upper left, so
/// the top and left walls are turned away from it and fall into shadow while the bottom and
/// right walls catch it. Painted before the label, so the label sits on the floor of the
/// recess with the wall showing around it.
const BEVEL: u32 = 3 * FACE_SCALE;

/// The grip ridge and the thumb notch, cut into the shell. Darkened rather than coloured:
/// moulded plastic is the same plastic, just turned away from the light.
fn mould_detail(face: &mut CartFace, shell: &Shell, size: CartSize, system: System) {
    let dark = [
        (shell.colour[0] as f32 * 0.62) as u8,
        (shell.colour[1] as f32 * 0.62) as u8,
        (shell.colour[2] as f32 * 0.62) as u8,
    ];
    for (px, cover) in face.rgba.chunks_exact_mut(4).zip(detail_mask(
        system,
        size.face_w(),
        size.face_h(),
    )) {
        let a = *cover as u32;
        if a == 0 {
            continue;
        }
        for c in 0..3 {
            px[c] = ((dark[c] as u32 * a + px[c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    }
}

fn recess_label(face: &mut CartFace, shell: &Shell, panel: (u32, u32, u32, u32)) {
    let (lx, ly, lw, lh) = panel;
    let (face_w, face_h) = (face.w, face.h);
    let shade = |c: [u8; 3], f: f32| -> [u8; 3] {
        [
            (c[0] as f32 * f).clamp(0.0, 255.0) as u8,
            (c[1] as f32 * f).clamp(0.0, 255.0) as u8,
            (c[2] as f32 * f).clamp(0.0, 255.0) as u8,
        ]
    };
    let dark = shade(shell.colour, 0.55);
    let lit = shade(shell.colour, 1.45);
    let (x0, y0) = (lx.saturating_sub(BEVEL), ly.saturating_sub(BEVEL));
    let (x1, y1) = (lx + lw + BEVEL, ly + lh + BEVEL);
    let clear = shell.finish != Finish::Solid;
    let mut put = |x: u32, y: u32, upper: bool| {
        if x >= face_w || y >= face_h {
            return;
        }
        let d = ((y * face_w + x) * 4) as usize;
        let c = if clear {
            let under = [face.rgba[d], face.rgba[d + 1], face.rgba[d + 2]];
            shade(under, if upper { 0.7 } else { 1.18 })
        } else if upper {
            dark
        } else {
            lit
        };
        face.rgba[d] = c[0];
        face.rgba[d + 1] = c[1];
        face.rgba[d + 2] = c[2];
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let inside = (lx..lx + lw).contains(&x) && (ly..ly + lh).contains(&y);
            if inside {
                continue;
            }
            let from_top = y.saturating_sub(y0);
            let from_left = x.saturating_sub(x0);
            let from_bottom = y1.saturating_sub(y + 1);
            let from_right = x1.saturating_sub(x + 1);
            let upper = from_top.min(from_left);
            let lower = from_bottom.min(from_right);
            put(x, y, upper <= lower);
        }
    }
}

/// Source over, so a label with an alpha channel shows the shell through it rather than
/// punching a hole in the cart.
///
/// Credited rather than trusted: `panel` is `(x, y, w, h)`, the label is `w * h` pixels, and
/// both are clipped to the face rather than assumed to fit inside it. The panel used to be
/// handed in as corners, which pasted a label a corner's width too wide and a corner's height
/// too tall, and the write past the end of the face took the whole frontend down before its
/// first frame — on any card with a game on it, so the machine came up as a boot loop. A
/// labelled cart drawn slightly wrong is a bug; a panic here is a handheld that cannot start.
fn paste_label(face: &mut CartFace, label: &[u8], panel: (u32, u32, u32, u32)) {
    let (lx, ly, lw, lh) = panel;
    let face_w = face.w;
    for y in 0..lh {
        if y + ly >= face.h {
            break;
        }
        for x in 0..lw {
            if x + lx >= face_w {
                break;
            }
            let s = ((y * lw + x) * 4) as usize;
            let Some(src) = label.get(s..s + 4) else {
                break;
            };
            let a = src[3] as u32;
            if a == 0 {
                continue;
            }
            let d = (((y + ly) * face_w + x + lx) * 4) as usize;
            if d + 4 > face.rgba.len() {
                continue;
            }
            for c in 0..3 {
                face.rgba[d + c] =
                    ((src[c] as u32 * a + face.rgba[d + c] as u32 * (255 - a) + 127) / 255) as u8;
            }
        }
    }
}

/// The card's own cart art, when it supplies any for this machine. The label it gets is set at
/// the size of the slot the art marks out rather than at the built-in one, because the art is
/// the thing that says where its label goes.
fn card_face(cart: &Cart, shell: &Shell, size: CartSize) -> Option<CartFace> {
    let art = crate::cart_art::art_for(cart.system())?;
    let (lw, lh) = art.label_size(size)?;
    let label = match cart.label.as_deref().and_then(|p| art::cover(p, lw, lh)) {
        Some(rgba) => rgba,
        None => sized_label(&label_text(cart), lw, lh),
    };
    // What the cart becomes on the light theme. A Game Boy or Game Boy Color cart keeps its own
    // colour there; the Advance cart is repainted in the housing's own colour — the off-white the
    // top and bottom bands are drawn in — so the cart belongs to the same device as the furniture
    // around it. Taken from `housing()` rather than written down, so the two cannot drift apart.
    let light = match cart.system() {
        System::Gba => {
            let h = crate::slot_chrome::housing();
            Some([(h[0] * 255.0) as u8, (h[1] * 255.0) as u8, (h[2] * 255.0) as u8])
        }
        _ => None,
    };
    Some(art.face(shell, &label, lw, lh, size.face_w(), size.face_h(), light))
}

/// The generated label at any size. The built-in label is one call with its own constants; a
/// card-supplied cart asks for whatever its slot measures. Everything that decides how the type
/// sets — the padding, the ceiling on the point size, the floor under it — is the built-in
/// label's own figure scaled by how much taller or shorter this slot is, so a small slot gets
/// type in proportion rather than type that will not fit.
fn sized_label(title: &str, w: u32, h: u32) -> Vec<u8> {
    let k = h as f32 / LABEL_H as f32;
    let pad = (PAD as f32 * k) as u32;
    let bg = label_colour(title);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..w * h {
        rgba.extend_from_slice(&[bg[0], bg[1], bg[2], 255]);
    }

    if let Some(font) = text::label_font() {
        let layout = text::fit_box(
            font,
            title,
            w.saturating_sub(2 * pad) as f32,
            h as f32,
            MAX_LINES,
            MAX_PX * k,
            (MIN_PX * k).max(6.0),
        );
        text::draw_centred(&mut rgba, w, h, &layout, ink(bg));
    }
    rgba
}

fn ink(bg: [u8; 3]) -> [u8; 3] {
    let luma = 0.2126 * bg[0] as f32 + 0.7152 * bg[1] as f32 + 0.0722 * bg[2] as f32;
    if luma > 140.0 {
        [0x1a, 0x18, 0x16]
    } else {
        [0xf4, 0xf1, 0xea]
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}
