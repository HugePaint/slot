use crate::draw::{Draw, TexId, OUT_W};
use crate::text;

pub const HINT_H: u32 = 24;
pub const CAP: u32 = 20;
const CAP_PAD: u32 = 4;
const CAP_MAX_W: f32 = 64.0;
pub const HINT_GAP: f32 = 14.0;

pub const CAP_GAP: u32 = 5;
const EDGE: u32 = 2;
pub const HINT_EDGE: u32 = EDGE;
const LABEL_MAX_W: f32 = 140.0;

pub const TITLE_W: u32 = 360;
pub const TITLE_H: u32 = 24;

/// A cap is a filled square with its key cut out of it, which is what a key looks like and the
/// only thing separating the key from its label at this size. The fill is the ink and the cut
/// is the ink's opposite, in whichever mode — see `palette::panel_ink`.
fn cap_ink() -> [u8; 3] {
    crate::palette::panel_ink()
}

fn ink() -> [u8; 3] {
    crate::palette::ink()
}

const KEY_PX: f32 = 14.0;
const LABEL_PX: f32 = 16.0;
const LABEL_MIN_PX: f32 = 10.0;
/// The status row's own band height: the clock and the battery's percent share it.
///
/// The two readings are set at `CLOCK_PX`/`PCT_PX`, sized so the face's own line box fits inside
/// this band. `text::coverage` places the baseline from the box's height and the face's own line
/// metrics, and then its bounds check simply *drops* any ink past the edge — so a font too large
/// for the band loses the bottom of its digits rather than shrinking to fit. `text::fit` has an
/// opinion about width only, and none at all about height.
///
/// Only the **box** is this tall. Where the ink lands inside it is decided by the face's own line
/// metrics (`text::coverage`).
pub const STATUS_H: u32 = 20;

/// The clock's size. Set to sit inside the `STATUS_H` band: twenty-four keeps the baseline just
/// inside the box's bottom, where thirty pushed it past and clipped the digits' lower edge.
const CLOCK_PX: f32 = 24.0;
const CLOCK_MIN_PX: f32 = 12.0;

/// The battery's percent, set at the clock's own size: they are the two readings on the status
/// row and were asked to grow together. Its own pair rather than a share of `LABEL_PX`, which
/// belongs to hint labels and must stay where the legends were measured.
const PCT_PX: f32 = 24.0;
const PCT_MIN_PX: f32 = 12.0;
const TITLE_PX: f32 = 20.0;
const TITLE_MIN_PX: f32 = 12.0;

/// The shelf's own line of type: which cart is under the eye, set below the row. Its own size
/// rather than the top plate's, because it is the only thing on that screen naming the
/// selection — the cart carries a printed label, but that label is a 240 px cart's, and a
/// second reading at the size of a caption on the case would not be a second reading at all.
///
/// One line, never two: a title that wraps moves its own bottom line every time the row
/// moves, and the row is the thing being read.
///
/// The box grew with the type when the type was asked to. `H` is the box the string is
/// rasterised into and `PX` is what it is set at, so a size that outgrew its box would be a
/// clipped title rather than a failing test: the fitter shrinks for *width* and has no opinion
/// about height at all. Forty is thirty plus five of air either side, which is what a CJK glyph
/// at this size needs to keep its top and bottom strokes.
pub const SHELF_TITLE_W: u32 = 640;
pub const SHELF_TITLE_H: u32 = 40;
const SHELF_TITLE_PX: f32 = 30.0;
const SHELF_TITLE_MIN_PX: f32 = 14.0;

pub struct UndoFace {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hint {
    pub key: &'static str,
    pub label: String,
}

pub fn hint_row(legend: &[(&'static str, &'static str)]) -> Vec<Hint> {
    legend
        .iter()
        .map(|(key, label)| Hint {
            key,
            label: label.to_string(),
        })
        .collect()
}

pub fn cap_width(key: &str) -> u32 {
    let Some(font) = text::label_font() else {
        return CAP;
    };
    let layout = text::fit(font, key, CAP_MAX_W, 1, KEY_PX, KEY_PX);
    let ink = layout
        .lines
        .iter()
        .map(|l| text::line_width(font, l, layout.px, layout.tracking))
        .fold(0.0, f32::max);
    CAP.max(ink.ceil() as u32 + 2 * CAP_PAD)
}

pub fn hint_width(key: &str, label: &str) -> u32 {
    cap_width(key) + CAP_GAP + band_width(label) + EDGE
}

pub fn hint_face(key: &str, label: &str) -> UndoFace {
    let w = hint_width(key, label);
    let mut rgba = vec![0u8; (w * HINT_H * 4) as usize];

    let cap_w = cap_width(key);
    let mut cap = Vec::with_capacity((cap_w * CAP * 4) as usize);
    for _ in 0..cap_w * CAP {
        cap.extend_from_slice(&[ink()[0], ink()[1], ink()[2], 255]);
    }
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, key, cap_w as f32, 1, KEY_PX, KEY_PX);
        text::draw_centred(&mut cap, cap_w, CAP, &layout, cap_ink());
    }
    blit(&mut rgba, w, &cap, cap_w, CAP, 0, (HINT_H - CAP) / 2);

    let text_w = band_width(label);
    let mut band = vec![0u8; (text_w * HINT_H * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, label, text_w as f32, 1, LABEL_PX, LABEL_MIN_PX);
        text::draw_centred(&mut band, text_w, HINT_H, &layout, ink());
    }
    blit(&mut rgba, w, &band, text_w, HINT_H, cap_w + CAP_GAP, 0);

    UndoFace { rgba, w, h: HINT_H }
}

pub fn word_width(text: &str) -> u32 {
    band_width_at(text, PCT_PX, PCT_MIN_PX)
}

/// The battery's percent. Named `word_*` for history and now the only caller; it is set at
/// `PCT_PX` in a `STATUS_H` band, so it and the clock are the same size in the same box.
pub fn word_face(text: &str) -> UndoFace {
    let w = word_width(text);
    let mut rgba = vec![0u8; (w * STATUS_H * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, text, w as f32, 1, PCT_PX, PCT_MIN_PX);
        text::draw_centred(&mut rgba, w, STATUS_H, &layout, ink());
    }
    UndoFace {
        rgba,
        w,
        h: STATUS_H,
    }
}

/// The time. Same face and same band as the battery's percent — the two halves of one status row.
pub fn clock_width(text: &str) -> u32 {
    band_width_at(text, CLOCK_PX, CLOCK_MIN_PX)
}

pub fn clock_face(text: &str) -> UndoFace {
    let w = clock_width(text);
    let mut rgba = vec![0u8; (w * STATUS_H * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, text, w as f32, 1, CLOCK_PX, CLOCK_MIN_PX);
        text::draw_centred(&mut rgba, w, STATUS_H, &layout, ink());
    }
    UndoFace {
        rgba,
        w,
        h: STATUS_H,
    }
}

pub fn hint_quad(x: f32, y: f32, w: f32, face: Option<TexId>) -> Draw {
    let h = HINT_H as f32;
    match face {
        Some(tex) => Draw::Tex {
            x,
            y,
            w,
            h,
            tex,
            alpha: 1.0,
        },
        None => Draw::Rect {
            x,
            y,
            w,
            h,
            colour: [1.0, 1.0, 1.0, 0.12],
        },
    }
}

pub const LEGEND_GAP: f32 = 36.0;

pub fn centred_hints(hints: &[(TexId, u32)], gap: f32) -> Vec<(TexId, u32, f32)> {
    let seen = |w: u32| w.saturating_sub(HINT_EDGE) as f32;
    let total = hints.iter().map(|&(_, w)| seen(w)).sum::<f32>()
        + gap * hints.len().saturating_sub(1) as f32;
    let mut x = ((OUT_W as f32 - total) / 2.0).round();
    hints
        .iter()
        .map(|&(tex, w)| {
            let at = x.round();
            x += seen(w) + gap;
            (tex, w, at)
        })
        .collect()
}

fn band_width(label: &str) -> u32 {
    band_width_at(label, LABEL_PX, LABEL_MIN_PX)
}

/// The same, at a stated size. The clock and the battery's percent are one band of type rendered
/// twice at two sizes, and this is the one place the size is a parameter rather than a constant —
/// which is what keeps their widths, their shrink thresholds and their fitter rules from drifting
/// apart the moment one of the two sizes is touched.
fn band_width_at(label: &str, px: f32, min_px: f32) -> u32 {
    let Some(font) = text::label_font() else {
        return LABEL_MAX_W as u32;
    };
    let layout = text::fit(font, label, LABEL_MAX_W, 1, px, min_px);
    let ink = layout
        .lines
        .iter()
        .map(|l| text::line_width(font, l, layout.px, layout.tracking))
        .fold(0.0, f32::max);
    (ink.ceil() as u32).clamp(1, LABEL_MAX_W as u32)
}

pub fn title_face(text: &str) -> UndoFace {
    let mut rgba = vec![0u8; (TITLE_W * TITLE_H * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, text, TITLE_W as f32, 1, TITLE_PX, TITLE_MIN_PX);
        text::draw_centred(&mut rgba, TITLE_W, TITLE_H, &layout, ink());
    }
    UndoFace {
        rgba,
        w: TITLE_W,
        h: TITLE_H,
    }
}

/// One line of the direct-connect dialog's own type. Its own builder rather than a share of a
/// screen's, because the dialog is a screen of its own and asks for sizes (36/24) nothing else
/// prints at. The box is twice the type's height so the font's own line box fits rather than being
/// clipped — `text::coverage` drops ink past the box's edge instead of shrinking it.
pub fn dialog_line_face(text: &str, px: f32) -> UndoFace {
    let max_w = 640.0f32;
    let box_h = (px * 2.0).ceil() as u32;
    let mut w = 1u32;
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, text, max_w, 1, px, px * 0.5);
        let ink = layout
            .lines
            .iter()
            .map(|l| text::line_width(font, l, layout.px, layout.tracking))
            .fold(0.0, f32::max);
        w = (ink.ceil() as u32).clamp(1, max_w as u32);
    }
    let mut rgba = vec![0u8; (w * box_h * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, text, w as f32, 1, px, px * 0.5);
        text::draw_centred(&mut rgba, w, box_h, &layout, ink());
    }
    UndoFace { rgba, w, h: box_h }
}

/// The shelf's line of type: the game under the eye, for the screen below the row. Cut from
/// the cart's own name rather than the raw filename, so what is set here and what is printed
/// on the cart are the same words — `slot_store::clean_label` is the one place that decides which
/// part of a dumped filename is the game and which part is a fact about the dump.
pub fn shelf_title_face(text: &str) -> UndoFace {
    let mut rgba = vec![0u8; (SHELF_TITLE_W * SHELF_TITLE_H * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(
            font,
            text,
            SHELF_TITLE_W as f32,
            1,
            SHELF_TITLE_PX,
            SHELF_TITLE_MIN_PX,
        );
        text::draw_centred(&mut rgba, SHELF_TITLE_W, SHELF_TITLE_H, &layout, ink());
    }
    UndoFace {
        rgba,
        w: SHELF_TITLE_W,
        h: SHELF_TITLE_H,
    }
}

pub(crate) fn blit(dst: &mut [u8], dst_w: u32, src: &[u8], src_w: u32, src_h: u32, x: u32, y: u32) {
    for row in 0..src_h {
        let from = ((row * src_w) * 4) as usize;
        let to = (((y + row) * dst_w + x) * 4) as usize;
        dst[to..to + (src_w * 4) as usize].copy_from_slice(&src[from..from + (src_w * 4) as usize]);
    }
}

pub const ARROW_GAP: u32 = 3;

const LEFT_CARET: char = '\u{f0d9}';
const RIGHT_CARET: char = '\u{f0da}';

pub fn arrows_hint_width(label: &str) -> u32 {
    2 * CAP + ARROW_GAP + CAP_GAP + band_width(label) + EDGE
}

pub fn arrows_hint_face(label: &str) -> UndoFace {
    let w = arrows_hint_width(label);
    let mut rgba = vec![0u8; (w * HINT_H * 4) as usize];
    for (i, glyph) in [LEFT_CARET, RIGHT_CARET].into_iter().enumerate() {
        blit(
            &mut rgba,
            w,
            &glyph_cap(glyph),
            CAP,
            CAP,
            i as u32 * (CAP + ARROW_GAP),
            (HINT_H - CAP) / 2,
        );
    }
    let text_w = band_width(label);
    let mut band = vec![0u8; (text_w * HINT_H * 4) as usize];
    if let Some(font) = text::label_font() {
        let layout = text::fit(font, label, text_w as f32, 1, LABEL_PX, LABEL_MIN_PX);
        text::draw_centred(&mut band, text_w, HINT_H, &layout, ink());
    }
    blit(
        &mut rgba,
        w,
        &band,
        text_w,
        HINT_H,
        2 * CAP + ARROW_GAP + CAP_GAP,
        0,
    );
    UndoFace { rgba, w, h: HINT_H }
}

fn glyph_cap(glyph: char) -> Vec<u8> {
    let mut cap = Vec::with_capacity((CAP * CAP * 4) as usize);
    for _ in 0..CAP * CAP {
        cap.extend_from_slice(&[ink()[0], ink()[1], ink()[2], 255]);
    }
    let Some(font) = crate::icon::symbols_font() else {
        return cap;
    };
    let (m, cov) = font.rasterize(glyph, KEY_PX);
    let x0 = (CAP as i32 - m.width as i32) / 2;
    let y0 = (CAP as i32 - m.height as i32) / 2;
    for gy in 0..m.height {
        for gx in 0..m.width {
            let (dx, dy) = (x0 + gx as i32, y0 + gy as i32);
            if dx < 0 || dy < 0 || dx >= CAP as i32 || dy >= CAP as i32 {
                continue;
            }
            let a = cov[gy * m.width + gx] as u32;
            let at = ((dy as u32 * CAP + dx as u32) * 4) as usize;
            for k in 0..3 {
                cap[at + k] =
                    ((cap_ink()[k] as u32 * a + cap[at + k] as u32 * (255 - a)) / 255) as u8;
            }
        }
    }
    cap
}
