use std::sync::OnceLock;

use slot_gfx::{Draw, TexId, OUT_H, OUT_W};
use slot_store::Cart;
use slot_store::Theme;

use crate::cart::{label_colour, label_text, size_for, SEATED_W};
use crate::icon::icon_box;
use crate::palette;
use crate::plate::HINT_H;
use crate::shelf::CENTER_SCALE;
use crate::status::Printed;

pub const ALERT_PX: f32 = 44.0;

pub const MOUTH_H: f32 = 58.0;
const BAND_Y: f32 = OUT_H as f32 - MOUTH_H;

pub const MOUTH_W: f32 = SEATED_W as f32 + 14.0;
const SLIT_H: f32 = 9.0;
const MOUTH_X: f32 = (OUT_W as f32 - MOUTH_W) / 2.0;
const SLIT_Y: f32 = BAY_Y + 5.0;

pub const LIP_H: f32 = 2.0;

const BAY_W: f32 = MOUTH_W + 18.0;
const BAY_X: f32 = (OUT_W as f32 - BAY_W) / 2.0;
const BAY_Y: f32 = BAND_Y + LIP_H;
const SCOOP_W: f32 = MOUTH_W * 0.88;
const SCOOP_D: f32 = RECESS_H - (SCOOP_Y - BAY_Y);
const SCOOP_Y: f32 = SLIT_Y + SLIT_H;

const RECESS_H: f32 = 42.0;
const RIM_W: f32 = 2.0;
const CX: f32 = OUT_W as f32 / 2.0;
const SCOOP_FLAT: f32 = 4.0;

static THEME: OnceLock<Theme> = OnceLock::new();

pub fn set_theme(theme: Theme) {
    let _ = THEME.set(theme);
}

pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::default)
}

fn rgb(c: [u8; 3]) -> [f32; 4] {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        1.0,
    ]
}

/// One surface of the case, in whichever palette the device is in.
///
/// The card's `theme.txt` is read once into `THEME` and is the *dark* case; the light case is
/// the built-in one. Every accessor below goes through here, so there is exactly one place
/// that knows a mode exists and no way for one band of the case to be left behind when it
/// changes — which is the failure a per-band `if` would produce eventually.
fn surface(pick: fn(&Theme) -> [u8; 3]) -> [f32; 4] {
    match palette::mode() {
        palette::Mode::Dark => rgb(pick(theme())),
        palette::Mode::Light => rgb(pick(&palette::LIGHT_CASE)),
    }
}

/// The case. Every band has to clear its neighbour, which `slot-ui/tests/contrast.rs` holds
/// for both palettes. A theme is the card's own business past that.
pub fn housing() -> [f32; 4] {
    surface(|t| t.housing)
}

pub fn opening() -> [f32; 4] {
    surface(|t| t.opening)
}

pub fn edge() -> [f32; 4] {
    surface(|t| t.edge)
}

pub fn recess() -> [f32; 4] {
    surface(|t| t.recess)
}

const LIP_Y: f32 = BAND_Y;

/// How far a Game Boy or Game Boy Color cart goes into an Advance slot before it stops. The
/// slot is shallow and the cart is tall, so it goes in and then stands there rather than being
/// swallowed: 184 px of travel, against the Advance cart's 255.
///
/// Two fifths of the shell, measured against the plastic rather than against the travel: the
/// bay's front edge cuts the cart off at y 424, and the cart stands at y 102, so 212 px in
/// leaves 424 - (102 + 212) = 110 px out of 276 showing — 39.9%.
const GB_INSET: f32 = 212.0;

/// Where an Advance cart stops. In means *in*, not gone: it comes to rest filling the opening,
/// so the base of the slot is covered by the cart rather than going dark again. Four pixels
/// below the top of the recess, which leaves the far wall showing above the cart's rounded
/// top edge instead of butting it flat against the lip. A Game Boy cart stops higher up — see
/// `draw`, which derives the rest and the catch from the cart's own size.
const SEATED_Y: f32 = BAY_Y + 4.0;
/// The seat either side of the catch. It opens a little before halfway because the cart is
/// resting on the lip for the whole of it, and the push comes after.
const CATCH_IN: f32 = 0.42;
const CATCH_OUT: f32 = 0.62;
const CREEP: f32 = 0.03;

pub struct SlotChrome<'a> {
    pub cart: &'a Cart,
    pub face: Option<TexId>,
    pub rest: f32,
    pub scale: f32,
    pub seat: f32,
    pub alert: Option<(TexId, f32)>,
    pub dim: f32,
    pub screen: f32,
    pub game: bool,
}

impl SlotChrome<'_> {
    pub fn draw(&self, out: &mut Vec<Draw>) {
        let seat = self.seat.clamp(0.0, 1.0);
        let dim = self.dim.clamp(0.0, 1.0);
        if dim > 0.0 {
            out.push(Draw::Rect {
                x: 0.0,
                y: 0.0,
                w: OUT_W as f32,
                h: OUT_H as f32,
                colour: [0.0, 0.0, 0.0, dim],
            });
        }

        let chrome = 1.0 - self.screen.clamp(0.0, 1.0);

        draw_slot_back(chrome, out);

        // The cart eases from the carousel's enlarged centre scale back to its own size as it
        // seats, so the size the row showed it at flows into the slot instead of popping.
        //
        // Anchored by its foot on the way down, because that is where the row stands it: the
        // shelf draws the hero with its foot on `FOOT_Y`, so the extra height a larger cart
        // carries has to push it *up*. Centring it instead lets the extra height out both ways
        // and the cart drops by half of it the moment the insert starts — a jump of its own,
        // which is the thing scaling it here was meant to avoid.
        // The cart's own geometry, which is its machine's: an Advance cart is landscape and a
        // Game Boy cart portrait, and the two are not the same object going into the same slot.
        let size = size_for(self.cart.system());
        let (w0, h0) = (size.w as f32, size.h as f32);
        let cart_x = (OUT_W as f32 - w0) / 2.0;
        let rest_y = (OUT_H as f32 - h0) / 2.0;
        // Where it stops. An Advance cart is swallowed to the top of the recess. A Game Boy or
        // Game Boy Color cart cannot be: the slot is not deep enough to take one, and in a real
        // Advance it stands half out with its label still readable. So its foot comes to rest
        // halfway down the recess rather than at the bottom of it — a Game Boy cart in an
        // Advance slot is half in and stays half out, on the shelf and in the machine alike.
        let seated_y = match self.cart.system() {
            slot_store::System::Gba => SEATED_Y,
            _ => rest_y + GB_INSET,
        };
        // The Advance cart eases out of the size the row showed it at. A portrait cart is never
        // drawn enlarged — the row shows it at its own size — so there is nothing to ease out
        // of and it comes down at that size for the whole of the travel.
        let cart_scale = match self.cart.system() {
            slot_store::System::Gba => CENTER_SCALE + (1.0 - CENTER_SCALE) * seat,
            _ => 1.0,
        };
        // How far into the travel its bottom edge reaches the lip. Derived per cart, since it
        // depends on how far that cart has to fall.
        let catch_at = ((LIP_Y - h0 - rest_y) / (seated_y - rest_y).max(1.0)).clamp(0.05, 0.95);
        let cart_w = w0 * cart_scale;
        let cart_h = h0 * cart_scale;
        let x = cart_x + (w0 - cart_w) / 2.0;
        let y = rest_y + (seated_y - rest_y) * travel(seat, catch_at) + (h0 - cart_h);
        // The cart fades with the case rather than through it. A seated cart is really in the
        // slot and has to be drawn, so the whole device face has to leave as one object as the
        // picture takes over. Held at full while the screen is off, which is all of the travel.
        let cart_alpha = if seat >= 1.0 { chrome } else { 1.0 };
        out.push(match self.face {
            Some(tex) => Draw::Tex {
                x,
                y,
                w: cart_w,
                h: cart_h,
                tex,
                alpha: cart_alpha,
            },
            None => {
                let c = label_colour(&label_text(self.cart));
                Draw::Rect {
                    x,
                    y,
                    w: cart_w,
                    h: cart_h,
                    colour: [
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        cart_alpha,
                    ],
                }
            }
        });

        if let Some((tex, alpha)) = self.alert {
            let (w, h) = icon_box(ALERT_PX);
            let (w, h) = (w as f32, h as f32);
            out.push(Draw::Tex {
                x: x + (cart_w - w) / 2.0,
                y: y + (cart_h - h) / 2.0,
                w,
                h,
                tex,
                alpha,
            });
        }

        if chrome < 1.0 {
            front_bands(|x, y, w, h, _| {
                out.push(Draw::Rect {
                    x,
                    y,
                    w,
                    h,
                    colour: [0.0, 0.0, 0.0, 1.0],
                });
            });
        }

        if self.game && self.screen > 0.0 {
            out.push(Draw::Game);
        }

        draw_slot_front(chrome, out);
    }
}

fn band(x: f32, y: f32, w: f32, h: f32, c: [f32; 4], alpha: f32) -> Draw {
    Draw::Rect {
        x,
        y,
        w,
        h,
        colour: [c[0], c[1], c[2], c[3] * alpha],
    }
}

fn draw_slot_back(alpha: f32, out: &mut Vec<Draw>) {
    out.push(band(BAY_X, BAND_Y, BAY_W, LIP_H, housing(), alpha));
    out.push(band(MOUTH_X, BAND_Y, MOUTH_W, LIP_H, edge(), alpha));
    out.push(band(BAY_X, BAY_Y, BAY_W, RECESS_H, recess(), alpha));
    out.push(band(MOUTH_X, SLIT_Y, MOUTH_W, SLIT_H, opening(), alpha));
    for_each_scoop_span(|x, w, depth| {
        out.push(band(x, SCOOP_Y, w, depth, opening(), alpha));
    });
}

fn draw_slot_front(alpha: f32, out: &mut Vec<Draw>) {
    front_bands(|x, y, w, h, c| out.push(band(x, y, w, h, c, alpha)));
}

fn front_bands(mut band: impl FnMut(f32, f32, f32, f32, [f32; 4])) {
    let w = OUT_W as f32;
    let right = BAY_X + BAY_W;
    let floor = SCOOP_Y + SCOOP_D + RIM_W;
    band(0.0, BAND_Y, BAY_X, MOUTH_H, housing());
    band(right, BAND_Y, w - right, MOUTH_H, housing());

    let near = CX - SCOOP_W / 2.0;
    band(BAY_X, SCOOP_Y, near - BAY_X, floor - SCOOP_Y, housing());
    let far = CX + SCOOP_W / 2.0;
    band(far, SCOOP_Y, right - far, floor - SCOOP_Y, housing());

    for_each_scoop_span(|x, w, depth| {
        let top = SCOOP_Y + depth + RIM_W;
        band(x, top, w, floor - top, housing());
    });
    band(0.0, floor, w, OUT_H as f32 - floor, housing());
    for_each_scoop_span(|x, w, depth| {
        band(x, SCOOP_Y + depth, w, RIM_W, edge());
    });
}

fn for_each_scoop_span(mut span: impl FnMut(f32, f32, f32)) {
    let hw = SCOOP_W / 2.0;
    let depth = |x: f32| SCOOP_D * (1.0 - (x.abs() / hw).powf(SCOOP_FLAT)).max(0.0);
    let mut x = -hw;
    while x < hw {
        let d = depth(x + 0.5).round();
        let start = x;
        while x < hw && depth(x + 0.5).round() == d {
            x += 1.0;
        }
        span(CX + start, x - start, d);
    }
}

// ---------------------------------------------------------------------------------------
// The top band: the same bay the other way up, with the letter strip in its window
// ---------------------------------------------------------------------------------------

/// The band the letter strip lives in, at the top of the screen. The same depth as the cart
/// bay, and the same width down to the pixel: what frames the letters is the machine's own
/// opening rather than a second piece of furniture invented for them, which is the whole of
/// why the strip reads as part of the device.
pub const TOP_BAND_H: f32 = MOUTH_H;

/// The window the letters show through: the bay's opening, mirrored. Its width is the
/// opening's and it stands on the same centre, so the strip above and the row below cannot
/// disagree about where the middle of the screen is.
pub const TOP_WIN_X: f32 = MOUTH_X;
pub const TOP_WIN_W: f32 = MOUTH_W;
pub const TOP_WIN_Y: f32 = 12.0;
pub const TOP_WIN_H: f32 = 36.0;

/// The well around it — the bay's recess, read the other way up — and so the depth the band
/// has over once the window has taken its share.
///
/// The well is not `RECESS_H`: the bay's opening is as deep as a thumb, because a hand has to
/// reach a cart's face through it, and this one is as deep as a line of letters. The window
/// sits in the well the way the opening sits in the recess, four pixels in from it.
const TOP_WELL_Y: f32 = 8.0;
const TOP_WELL_H: f32 = 44.0;

/// How far the light off a band reaches into the screen, how finely the falloff is cut, and how
/// bright it is where it starts.
///
/// The draw list is filled quads and nothing else — no gradient, no blur — so a glow is a stack
/// of one-pixel bands whose alpha steps down. Ten is fine enough that the steps do not read as
/// bands, which is the only thing that would give the trick away.
///
/// Faint on purpose, and it is worth saying why: this is the sheen of light spilling off a lit
/// edge onto the picture behind it, not a second edge. At any real strength it stops reading as
/// light and starts reading as a border someone drew on.
const GLOW_H: f32 = 10.0;
const GLOW_STEPS: usize = 10;
const GLOW_PEAK: f32 = 0.16;

/// The light that spills off one edge of the case onto the screen behind it.
///
/// `edge_y` is the boundary itself and `down` is which way the screen is from it: the band at
/// the top of the case throws its light downward, the bay at the bottom throws its upward, and
/// both are the same falloff read in opposite directions.
///
/// The falloff is squared rather than straight, because a linear ramp reads as a deliberate
/// gradient while a squared one reads as light: bright where it leaves the plastic, and gone
/// before the eye has followed it anywhere.
fn glow(out: &mut Vec<Draw>, edge_y: f32, down: bool) {
    let step = GLOW_H / GLOW_STEPS as f32;
    for i in 0..GLOW_STEPS {
        // One at the edge, zero at the end of the reach, and dimmer than that in between.
        let t = 1.0 - (i as f32 + 0.5) / GLOW_STEPS as f32;
        let alpha = GLOW_PEAK * t * t;
        if alpha <= 0.0 {
            continue;
        }
        // Stepped away from the edge, never onto it: the band's own lit edge is drawn already
        // and a glow across it would only wash it out.
        let y = if down {
            edge_y + i as f32 * step
        } else {
            edge_y - (i as f32 + 1.0) * step
        };
        out.push(band(0.0, y, OUT_W as f32, step, edge(), alpha));
    }
}

/// The light off both bands of the case: down from the top of the screen, up from the bay at
/// the bottom.
///
/// Both, and in one call, because they are one thing seen twice — the case is the same object
/// top and bottom, and the two boundaries are where it meets the picture. Drawn as the last
/// piece of the case's own furniture so a cart on its way into the slot is lit by it rather
/// than drawing over it.
pub fn draw_edge_glow(out: &mut Vec<Draw>) {
    glow(out, TOP_BAND_H, true);
    glow(out, BAND_Y, false);
}

/// The band at the top of the screen, drawn from the bay's numbers rather than from a second
/// set worked out for it.
///
/// Laid out as the bay is, read from the bottom up: the housing, the well stepped into it, the
/// opening inside that, and the lit edge of the cut at the top of the window — `RIM_W`.
///
/// **No lip, and that is a change from the bay rather than an omission.** The bay's own lower
/// boundary carries a lit edge — `LIP_H`, the line a cart is cut off at as it goes in — and the
/// band was first drawn with the same line under its window, on the argument that it was the
/// same moulding seen the other way up. It is not: the band is a lid, nothing passes through it,
/// and a second line two pixels above the band's edge read as a rule drawn under the letters
/// rather than as an edge of the case. The bay keeps its lip because a cart really is cut off
/// there.
///
/// One difference, and it is the reason this is a band and not a slot: nothing goes through
/// it. There is no depth behind the window and no shadow in it, because the letters are printed
/// on the inside of the opening and the band is a lid, so the whole of it is drawn flat.
pub fn draw_top_band(out: &mut Vec<Draw>) {
    out.push(band(0.0, 0.0, OUT_W as f32, TOP_BAND_H, housing(), 1.0));
    out.push(band(BAY_X, TOP_WELL_Y, BAY_W, TOP_WELL_H, recess(), 1.0));
    out.push(band(
        TOP_WIN_X,
        TOP_WIN_Y,
        TOP_WIN_W,
        TOP_WIN_H,
        opening(),
        1.0,
    ));
    // The lit edge last, so nothing is painted over the highlight — the order the bay uses.
    out.push(band(
        TOP_WIN_X,
        TOP_WIN_Y - RIM_W,
        TOP_WIN_W,
        RIM_W,
        edge(),
        1.0,
    ));
}

pub fn draw_slot_name(name: Printed, alpha: f32, out: &mut Vec<Draw>) {
    let (Some(tex), true) = (name.face, alpha > 0.0) else {
        return;
    };
    let (w, h) = (name.w as f32, HINT_H as f32);
    let hole = SCOOP_Y + SCOOP_D - SLIT_Y;
    out.push(Draw::Tex {
        x: CX - w / 2.0,
        y: SLIT_Y + (hole - h) / 2.0,
        w,
        h,
        tex,
        alpha,
    });
}

/// The slot with nothing going into it. The shelf shows it so the cart you pick has a
/// visible place to go, and so the bottom of the screen is the same object on every screen
/// rather than appearing only during the animation.
pub fn draw_empty_slot(out: &mut Vec<Draw>) {
    draw_slot_back(1.0, out);
    draw_slot_front(1.0, out);
}

/// The travel, in three parts: the cart falls to the lip, rests on it, then is pushed
/// through and settles. A single ease covers the same ground but arrives seated without ever
/// having met anything, which is what makes it read as a card going down a chute.
fn travel(seat: f32, catch_at: f32) -> f32 {
    if seat < CATCH_IN {
        catch_at * ease(seat / CATCH_IN)
    } else if seat < CATCH_OUT {
        catch_at + CREEP * (seat - CATCH_IN) / (CATCH_OUT - CATCH_IN)
    } else {
        let caught = catch_at + CREEP;
        caught + (1.0 - caught) * ease((seat - CATCH_OUT) / (1.0 - CATCH_OUT))
    }
}

pub fn ease(u: f32) -> f32 {
    u * u * u * (u * (u * 6.0 - 15.0) + 10.0)
}
