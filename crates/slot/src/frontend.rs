use std::time::{Duration, Instant};

use slot_gfx::{Compositor, Draw, ScreenEffect, TexId, OUT_H, OUT_W};

const PAPER: &[u8] = include_bytes!("../assets/paper.png");
use slot_input::{InputSource, Millis};
use slot_power::{Platform, Power};
use slot_store::{format_stamp, System};
use slot_ui::lang;
use slot_ui::{
    arrows_hint_face, badge_face, bezel_face, cart_face, cart_placeholder, cart_shadow,
    cheat_row_face, chip_face, chip_shadow_face, clean_label, clock_face, date_time_text,
    dialog_line_face, gb_cart_shadow, hhmm, hint_face, icon_face, letters, menu_face,
    palette_cell_w, palette_name_face, palette_name_h, photo_face, quick_caret_face,
    quick_label_face, quick_legend_faces, quick_value_face, rebuild_count_face, rebuild_title_face,
    set_clock_hint_face, shelf_title_face, shortcut_hint_face, shortcut_row_face, size_for,
    socket_face, sticker_face, title_face, toast_face, wallpaper_face, word_face, CartFace,
    GbShell, Icon, LinkBadge, PowerChoice, QuickMenuFaces, QuickRow, QuickValue, RebuildScreen,
    StickerFields, Toast, UndoFace, ALERT_PX, BOLT_PX, HUD_ICON_PX, LEGEND,
    PALETTE_FOOT_PX, SHORTCUT_ROWS,
};

use crate::app::{App, LinkRow, Phase};
use crate::build_info::Build;
use crate::cart_faces::CartFaces;
use crate::face_builder::{shelf_window, FaceBuilder, ShelfFaceFiller};
use crate::label_cache::{self, Rebuild};
use crate::link_art_builder::LinkArtBuilder;
use crate::link_screen::{LinkSprites, Sprite};
use crate::link_start::{LinkFail, LinkStep};
use crate::root;
use crate::session::Session;
use crate::wallpaper;

const DOZE_TIMEOUT: Duration = Duration::from_secs(180);

const ALERT_INK: [u8; 3] = [0xf0, 0xb4, 0x3c];

/// Carts either side of the caret that keep a face texture resident.
///
/// The shelf draws three. Twelve leaves room for a flick's overshoot and for the caret's own
/// neighbours to already be there when it lands, while bounding the shelf's memory at about
/// sixteen megabytes whatever the card holds — which is what makes a nine-hundred-game card
/// the same proposition as a hundred-game one.
const RESIDENT: usize = 12;

/// How far past `RESIDENT` a cart may sit before its texture is released. The slack is what
/// stops a caret parked on a boundary from releasing and rebuilding the same cart every step.
const RESIDENT_HYSTERESIS: usize = 3;

pub struct Frontend {
    session: Session,
    start: Instant,
    last: Instant,
    draws: Vec<Draw>,
    polaroid_texes: Vec<TexId>,
    title_tex: Option<TexId>,
    /// The shelf's own line of type, and the name it was built for. The same deal as the top
    /// plate's, on the screen that needs it most: one texture, rebuilt when the cart under the
    /// eye changes and at no other time.
    shelf_title_tex: Option<TexId>,
    shelf_titled: Option<String>,
    /// Builds the open cart's faces off the frame loop.
    faces: FaceBuilder,
    /// Builds the shelf's remaining cart faces off the frame loop, nearest the caret first.
    shelf_faces: ShelfFaceFiller,
    /// Carts whose face is not on the shelf yet. Ordered by the request, not by the list.
    shelf_missing: Vec<usize>,
    /// The caret position the resident window was last built for.
    resident_at: Option<usize>,
    /// Whether the boot log has the row's own account of itself yet: which cart is in each
    /// slot, and which of those is holding a face rather than the stand-in.
    row_logged: bool,
    cart_faces: CartFaces,
    rebuild: Option<Rebuild>,
    stale_check: Option<std::sync::mpsc::Receiver<Vec<slot_store::Cart>>>,
    ui_faces: Option<std::sync::mpsc::Receiver<UiFaces>>,
    rebuild_faces: RebuildFaces,
    link_art: LinkArtBuilder,
    link_art_done: bool,
    /// The cart last asked for.
    core_asked: Option<String>,
    core_board_tex: Option<TexId>,
    core_lid_tex: Option<TexId>,
    core_built: Option<String>,
    undo_tex: Option<TexId>,
    switcher: Switcher,
    clocks: Clocks,
    about: AboutFace,
    /// The palette browser's names. A page is read one of two ways and the fields are that pair:
    /// a colour machine's nine grades get a face each, so the page can be read across; a Game Boy's
    /// 343 palettes keep the one line at the foot of the panel, because their names are longer
    /// than a cell (see `App::palette_names`). Never both — the two are keyed differently and each
    /// clears itself on the machine that does not use it.
    palette_texes: Vec<Option<TexId>>,
    palette_tex: Option<TexId>,
    /// The pages both were last built for: the cell faces and the foot line change on a page turn
    /// and at no other time, so a caret moving inside a page mints nothing.
    palette_paged: Option<(System, usize)>,
    palette_named: Option<String>,
    /// Whether the first frame's uptime has been written to `boot.log`. One line, once, so the
    /// log says how long the device took to reach the shelf and which base it did it on.
    frame_logged: bool,
    quick_clock: QuickClock,
}

#[derive(Default)]
struct RebuildFaces {
    title: Option<(TexId, u32, u32)>,
    count: Option<TexId>,
    count_size: (u32, u32),
    shown: Option<usize>,
}

#[derive(Default)]
struct QuickClock {
    dim: Option<TexId>,
    lit: Option<TexId>,
    shown: String,
}

#[derive(Default)]
struct AboutFace {
    tex: Option<TexId>,
    battery: Option<u8>,
}

#[derive(Default)]
struct Clocks {
    line: Option<TexId>,
    hint: Option<TexId>,
    shelf: Option<TexId>,
    picked: Option<String>,
    shown: String,
    battery: String,
    battery_tex: Option<TexId>,
    platform: String,
    platform_tex: Option<TexId>,
}

#[derive(Default)]
struct Switcher {
    open: bool,
    titled: Option<String>,
}

impl Frontend {
    pub fn boot(platform: Box<dyn Platform>) -> Self {
        Self::boot_with(platform, crate::audio::opened_sink())
    }

    pub fn boot_with(platform: Box<dyn Platform>, sink: Box<dyn crate::audio::AudioSink>) -> Self {
        let now = Instant::now();
        let cart_faces = CartFaces::spawn(platform.root().to_path_buf());
        let mut session = Session::boot_with(platform.root().to_path_buf(), sink);
        crate::boot_time::mark("session");
        let stale_check =
            Self::check_labels(platform.root(), session.app().carts().iter().cloned().collect());
        session
            .app_mut()
            .set_power(Power::new(platform, DOZE_TIMEOUT));
        Frontend {
            session,
            start: now,
            last: now,
            draws: Vec::new(),
            polaroid_texes: Vec::new(),
            title_tex: None,
            shelf_title_tex: None,
            shelf_titled: None,
            faces: FaceBuilder::spawn(),
            shelf_faces: ShelfFaceFiller::spawn(),
            shelf_missing: Vec::new(),
            resident_at: None,
            row_logged: false,
            cart_faces,
            rebuild: None,
            stale_check,
            ui_faces: None,
            rebuild_faces: RebuildFaces::default(),
            link_art: LinkArtBuilder::spawn(),
            link_art_done: false,
            core_asked: None,
            core_board_tex: None,
            core_lid_tex: None,
            core_built: None,
            undo_tex: None,
            switcher: Switcher::default(),
            clocks: Clocks::default(),
            about: AboutFace::default(),
            palette_texes: Vec::new(),
            palette_tex: None,
            palette_paged: None,
            palette_named: None,
            frame_logged: false,
            quick_clock: QuickClock::default(),
        }
    }

    /// The card's own typeface, if it has one. `System/fonts/` belongs to the user, and the
    /// whole point of reading it rather than baking a face in is that a script the embedded one
    /// does not carry — CJK, most obviously — should not need a rebuild to appear.
    ///
    /// Silent on both failure paths on purpose. A card with no font file is the shipped
    /// configuration; a card with a broken one is that same configuration a moment later.
    /// Neither is worth a line on a device with no console and a boot budget to protect.
    fn load_font(&self) {
        let Some(path) = self.session.app().root().and_then(root::font_file) else {
            return;
        };
        if let Ok(bytes) = std::fs::read(&path) {
            slot_ui::text::set_font(bytes);
        }
    }

    fn upload_paper(compositor: &mut Compositor) {
        let mut dec = png::Decoder::new(std::io::Cursor::new(PAPER));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let Ok(mut reader) = dec.read_info() else {
            return;
        };
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let Ok(info) = reader.next_frame(&mut buf) else {
            return;
        };
        if info.color_type != png::ColorType::Grayscale || info.width != info.height {
            return;
        }
        compositor.set_paper(info.width, &buf[..info.buffer_size()]);
    }

    pub fn upload_faces(&mut self, compositor: &mut Compositor) {
        // Timed: this is the one part of the boot that scales with the number of games on the
        // card, and the number that says whether a card's covers or the shelf's faces are what
        // the wait is being spent on.
        let faces_at = Instant::now();
        // Before anything is rasterised, and so before anything has an opinion about type: the
        // card's face has to be in place for the first cart's label, not for the second frame.
        let font_at = Instant::now();
        self.load_font();
        // Its own line, because it is the one cost here that does not scale with anything:
        // the whole card font is read off the SD card and parsed before a glyph can be drawn.
        self.session
            .boot_note(&format!("font load {} ms", font_at.elapsed().as_millis()));
        // Every title on the card, checked once against both faces: a subset font goes stale
        // the moment a game is added, and a missing glyph fails silently as a blank.
        let mut missing: Vec<char> = Vec::new();
        for cart in self.session.app().carts() {
            for ch in slot_ui::text::missing_in(&slot_ui::label_text(cart)) {
                if !missing.contains(&ch) {
                    missing.push(ch);
                }
            }
        }
        if !missing.is_empty() {
            self.session.boot_note(&format!(
                "font missing {} chars: {}",
                missing.len(),
                missing.iter().collect::<String>()
            ));
        }
                let carts_at = Instant::now();
        // The carts the row is actually drawing — not a contiguous run of library indices around
        // the caret. On a machine shelf those two sets are not the same one: this card files its
        // three Game Boy carts at 27, 127 and 637 among nine hundred Advance ones, so a window
        // that walks the library builds nine hundred Advance carts and misses the three on
        // screen, and the row comes up as the blank stand-in. See `Shelf::draw_window`.
        let index = self.session.app().shelf_index();
        let count = self.session.app().carts().len();
        let window = self.session.app().shelf_draw_window();
        let window = if window.is_empty() {
            shelf_window(index, count)
        } else {
            window
        };
        let mut faces: Vec<Option<TexId>> = vec![None; count];
        for &i in &window {
            let f = cart_face(&self.session.app().carts()[i]);
            faces[i] = Some(compositor.create_texture(f.w, f.h, &f.rgba));
        }
        let fixed_at = Instant::now();
        self.session.boot_note(&format!(
            "cart faces ({} built of {}) {} ms",
            window.len(),
            count,
            carts_at.elapsed().as_millis()
        ));
        self.session.app_mut().set_faces(faces);
        self.shelf_missing = (0..count).filter(|i| !window.contains(&i)).collect();
        // The furniture, and the one group that has to be built again when the mode moves:
        // see `upload_fixed`.
        self.upload_fixed(compositor);
        // Everything above is one texture per fixed string or glyph: none of it scales with
        // the card, all of it has to be rasterised before the first frame, and it is the last
        // large block left in the boot once the font and the cart faces are accounted for.
        self.session.boot_note(&format!(
            "fixed faces {} ms",
            fixed_at.elapsed().as_millis()
        ));
        let wall_at = Instant::now();
        self.upload_wallpaper(compositor);
        // The card's wallpaper, decoded and scaled. A card whose wallpaper is a photograph
        // pays for it here, once, in the same way a label used to.
        self.session
            .boot_note(&format!("wallpaper {} ms", wall_at.elapsed().as_millis()));
        self.upload_faces_note(faces_at);
    }

    /// How tall the star a starred cart wears is drawn. Larger than the indicator: it is read
    /// at the size of the cart it sits above, where the indicator is a lamp in a corner.
    const FAV_STAR_PX: f32 = 36.0;
    /// How tall the shelf's indicator is drawn.
    const FAV_IND_PX: f32 = 36.0;
    /// The dark theme's star, and the only thing on the shelf that is neither the case's ink nor
    /// the palette's. A favourite is a fact about the cart rather than about which mode the
    /// frontend is in, so it stays the same colour in both — this one is what it is in the dark,
    /// where a star is drawn and not a heart.
    const FAV_INK: [u8; 3] = [0xff, 0xc4, 0x1e];
    /// The light theme draws a heart instead, because that is what the light cart's own sticker
    /// carries in its corner and the indicator has to match it.
    const HEART_INK: [u8; 3] = [0xf4, 0x79, 0x83];
    /// The same heart unlit: solid, and the case's grey rather than an outline, which on a pale
    /// shelf read as a hole rather than as a shape.
    const HEART_INK_QUIET: [u8; 3] = [0x3c, 0x3c, 0x3c];

    pub fn upload_boot_faces(&mut self, compositor: &mut Compositor) {
        Self::upload_paper(compositor);
        crate::boot_time::mark("paper");
        self.load_font();
        if self.rebuild.is_none() {
            self.cart_faces.sync(self.session.app_mut(), compositor);
        }
        crate::boot_time::mark("cart faces");
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("slot-ui-faces".into())
            .spawn(move || {
                let _ = tx.send(UiFaces::build());
            });
        match spawned {
            Ok(_) => self.ui_faces = Some(rx),
            Err(_) => self.place_ui_faces(compositor, UiFaces::build()),
        }
        self.upload_fixed(compositor);
        let hint = set_clock_hint_face();
        self.clocks.hint = Some(compositor.create_texture(hint.w, hint.h, &hint.rgba));
        let notched = gb_cart_shadow(GbShell::Notched);
        let notched = compositor.create_texture(notched.w, notched.h, &notched.rgba);
        let rounded = gb_cart_shadow(GbShell::Rounded);
        let rounded = compositor.create_texture(rounded.w, rounded.h, &rounded.rgba);
        self.session.app_mut().set_gb_cart_shadows(notched, rounded);
        crate::boot_time::mark("shelf faces");
        self.upload_wallpaper(compositor);
    }

    fn upload_fixed(&mut self, compositor: &mut Compositor) {
        let icons = Icon::ALL
            .iter()
            .map(|i| {
                let f = icon_face(*i, HUD_ICON_PX, slot_ui::palette::ink());
                compositor.create_texture(f.w, f.h, &f.rgba)
            })
            .collect();
        self.session.app_mut().set_icon_faces(icons);
        // The favourites: the star a starred cart wears, and the shelf's indicator, unlit and
        // lit. Uploaded here with the rest of the fixed furniture, because none of the three
        // ever changes — the indicator lights by drawing the other face, not by re-rasterising.
        //
        // Two shapes rather than one, and that is what the mode decides. The dark shelf draws a
        // star, which is what the cart art's own corner carries there; the light shelf draws the
        // heart the light cart art has, so a light-mode cart and its indicator are the same mark.
        // `rebake` runs this again when the mode moves, which is where "fixed" ends.
        let (star, off, on) = match slot_ui::palette::mode() {
            slot_ui::palette::Mode::Dark => {
                let star = icon_face(Icon::Star, Self::FAV_STAR_PX, Self::FAV_INK);
                let star = (
                    compositor.create_texture(star.w, star.h, &star.rgba),
                    star.w,
                    star.h,
                );
                let off = icon_face(Icon::StarOutline, Self::FAV_IND_PX, Self::FAV_INK);
                let off = (
                    compositor.create_texture(off.w, off.h, &off.rgba),
                    off.w,
                    off.h,
                );
                let on = icon_face(Icon::Star, Self::FAV_IND_PX, Self::FAV_INK);
                let on = (compositor.create_texture(on.w, on.h, &on.rgba), on.w, on.h);
                (star, off, on)
            }
            slot_ui::palette::Mode::Light => {
                let star = icon_face(Icon::Heart, Self::FAV_STAR_PX, Self::HEART_INK);
                let star = (
                    compositor.create_texture(star.w, star.h, &star.rgba),
                    star.w,
                    star.h,
                );
                // Solid grey, not an outline: the cart's own corner heart is solid, and an
                // outline beside it read as a different, emptier mark.
                let off = icon_face(Icon::Heart, Self::FAV_IND_PX, Self::HEART_INK_QUIET);
                let off = (
                    compositor.create_texture(off.w, off.h, &off.rgba),
                    off.w,
                    off.h,
                );
                let on = icon_face(Icon::Heart, Self::FAV_IND_PX, Self::HEART_INK);
                let on = (compositor.create_texture(on.w, on.h, &on.rgba), on.w, on.h);
                (star, off, on)
            }
        };
        self.session.app_mut().set_fav_faces(star, off, on);
        // The three machine logos for the shelf indicator, bottom-left, in `GB, GBC, GBA`
        // order. Text for now — the user will bake real machine logos — uploaded here with the
        // rest of the fixed furniture so nothing is rasterised mid-swap.
        // A card that supplies its own badge is used as it is; a card that does not gets the
        // built-in word. Either way the face comes with its size, because the app fits it into
        // the bar's box rather than drawing it at the size it happens to be.
        let carts_dir = self.session.app().root().map(|r| r.join("System/Carts"));
        let shelf_ind: Vec<(TexId, u32, u32)> = [
            ("GB", "type_gb.png"),
            ("GBC", "type_gbc.png"),
            ("GBA", "type_gba.png"),
        ]
        .iter()
        .map(|(word, file)| {
            match carts_dir
                .as_deref()
                .and_then(|d| slot_ui::decode_rgba(&d.join(file)))
            {
                Some((rgba, w, h)) => (compositor.create_texture(w, h, &rgba), w, h),
                None => {
                    let f = word_face(word);
                    (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
                }
            }
        })
        .collect();
        self.session.app_mut().set_shelf_ind_faces(shelf_ind);
        // The light theme's three, drawn separately by the card rather than tinted from the dark
        // ones. A card without them falls back to the built-in word as well, so a switch to
        // light mode on such a card changes nothing but the rest of the palette.
        let shelf_ind_light: Vec<(TexId, u32, u32)> = [
            ("GB", "type_gb_light.png"),
            ("GBC", "type_gbc_light.png"),
            ("GBA", "type_gba_light.png"),
        ]
        .iter()
        .map(|(word, file)| {
            match carts_dir
                .as_deref()
                .and_then(|d| slot_ui::decode_rgba(&d.join(file)))
            {
                Some((rgba, w, h)) => (compositor.create_texture(w, h, &rgba), w, h),
                None => {
                    let f = word_face(word);
                    (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
                }
            }
        })
        .collect();
        self.session.app_mut().set_shelf_ind_light(shelf_ind_light);
        // Its own upload rather than one of the HUD's: it is drawn on a cart, at its own
        // size, and in a warning colour the level glyphs have no business borrowing.
        let alert = icon_face(Icon::Alert, ALERT_PX, ALERT_INK);
        let alert = compositor.create_texture(alert.w, alert.h, &alert.rgba);
        self.session.app_mut().set_alert_face(alert);
        // Uploaded at boot like everything else: a shutdown is the one moment there is no
        // time to rasterise anything, and the GPU is about to be taken away. One line per
        // choice, in `PowerChoice::ALL` order, at the menu's own size so the screen that
        // follows a choice is set in the same voice as the row that was chosen.
        let lines = PowerChoice::ALL
            .iter()
            .map(|c| {
                let f = menu_face(match c {
                    PowerChoice::Restart => lang::SHUTDOWN_RESTART,
                    PowerChoice::PowerOff => lang::SHUTDOWN_POWER_OFF,
                });
                (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
            })
            .collect();
        self.session.app_mut().set_shutdown_faces(lines);
        let menu = PowerChoice::ALL
            .iter()
            .map(|c| {
                let f = menu_face(c.text());
                (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
            })
            .collect();
        self.session.app_mut().set_power_menu_faces(menu);
        // The open cart's parts that never change: each socket, the chip seated in each, the
        // blank chip in flight and its shadow, in `Core::ALL` order. At boot like the power
        // menu's rows, so the first frame of a lid coming off is not spent in a rasteriser.
        let sockets = slot_store::Core::ALL
            .iter()
            .map(|c| {
                let f = socket_face(*c);
                compositor.create_texture(f.w, f.h, &f.rgba)
            })
            .collect();
        let chips = slot_store::Core::ALL
            .iter()
            .map(|c| {
                let f = chip_face(Some(*c));
                compositor.create_texture(f.w, f.h, &f.rgba)
            })
            .collect();
        let blank = chip_face(None);
        let blank = compositor.create_texture(blank.w, blank.h, &blank.rgba);
        let shadow = chip_shadow_face();
        let shadow = compositor.create_texture(shadow.w, shadow.h, &shadow.rgba);
        self.session
            .app_mut()
            .set_core_part_faces(sockets, chips, blank, shadow);
        // Every action the picker takes, the way out first and the choice last, as the
        // switcher's legend is ordered.
        let legend = [
            hint_face("B", lang::CORE_CANCEL),
            arrows_hint_face(lang::CORE_SWAP),
            hint_face("A", lang::CORE_CHOOSE),
        ]
        .into_iter()
        .map(|f| (compositor.create_texture(f.w, f.h, &f.rgba), f.w))
        .collect();
        self.session.app_mut().set_core_legend_faces(legend);
        // The in-game menu, its two link rows, and the sentences the screen says while a
        // link is coming up or after it did not. All of it at the same size and through the
        // same rasteriser as the two menus above, because they are the same object — and all
        // of it at boot, because a link that is failing is the worst moment to be asking a
        // font for a sentence.
        let link = menu_faces(compositor, LinkRow::ALL.iter().map(|r| r.text()));
        self.session.app_mut().set_link_menu_faces(link);
        let steps = menu_faces(compositor, LinkStep::ALL.iter().map(|s| s.line()));
        self.session.app_mut().set_link_step_faces(steps);
        let fails = menu_faces(compositor, LinkFail::SHOWN.iter().map(|f| f.line()));
        self.session.app_mut().set_link_fail_faces(fails);
        let toasts = Toast::all()
            .iter()
            .map(|t| {
                let f = toast_face(*t);
                compositor.create_texture(f.w, f.h, &f.rgba)
            })
            .collect();
        self.session.app_mut().set_toast_faces(toasts);
        // The cheat table's "this cart has no codes" panel, rasterised at boot like the other
        // fixed strings so opening it never waits on a font.
        let empty = menu_face("NO CHEATS");
        let empty = (
            compositor.create_texture(empty.w, empty.h, &empty.rgba),
            empty.w,
            empty.h,
        );
        self.session.app_mut().set_cheat_empty_face(empty);
        let legend = legend_faces(compositor, &LEGEND);
        self.session.app_mut().set_legend_faces(legend);
        // Built for the shelf the device came up on. A shelf switch later in the session changes
        // the row's geometry under them, which `set_shelf_geometry` re-cuts.
        let sys = self.session.app().shelf_system();
        let size = size_for(sys);
        let shadow = cart_shadow(size, sys);
        let id = compositor.create_texture(shadow.w, shadow.h, &shadow.rgba);
        if let Some(old) = self.session.app_mut().set_cart_shadow(id) {
            compositor.release_texture(old);
        }
        // The blank cart every not-yet-built face is drawn as. A jump across the alphabet
        // crosses carts no filler will ever reach in time, and this is what slides past in
        // their place — a cartridge, rather than the label's colour as a bar of paint.
        let blank = cart_placeholder(size, sys);
        let blank = compositor.create_texture(blank.w, blank.h, &blank.rgba);
        if let Some(old) = self.session.app_mut().set_cart_placeholder(blank) {
            compositor.release_texture(old);
        }
        // `draw_gauge` now draws the bolt beside the capsule, on the housing, in its own
        // reserved slot rather than over the fill. The housing tint was only ever needed to
        // hide the bolt inside the fill it sat on; out here it sits where every other HUD
        // glyph does, so it takes the same ink they do.
        let bolt = icon_face(Icon::Charging, BOLT_PX, slot_ui::palette::ink());
        let bolt_id = compositor.create_texture(bolt.w, bolt.h, &bolt.rgba);
        self.session.app_mut().set_bolt_face(bolt_id);
        // The letter strip: one face per slot of a fixed alphabet — twenty-seven small
        // textures against the cart faces' one apiece — and the one piece of metal the strip
        // repeats between them. None of them can ever change, so they are built here with the
        // rest of the fixed furniture rather than lazily the way a cart's is.
        //
        // The ridge is the only *texture* the strip carries: the band the letters stand in is
        // the machine's own, drawn from the cart bay's numbers every frame by `slot_chrome`.
        let letter_faces = letters::SLOTS
            .iter()
            .map(|ch| {
                let f = letters::letter_face(*ch);
                (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
            })
            .collect();
        self.session.app_mut().set_letter_faces(letter_faces);
        let ridge = letters::ridge_face();
        let ridge = (
            compositor.create_texture(ridge.w, ridge.h, &ridge.rgba),
            ridge.w,
            ridge.h,
        );
        self.session.app_mut().set_letter_ridge_face(ridge);
        // The shortcut card, in the same breath as the letters: twenty-odd rows of fixed
        // string, none of which can ever change, and opening a help screen is the worst moment
        // to be asking a font for them.
        let rows = SHORTCUT_ROWS
            .iter()
            .map(|r| {
                let f = shortcut_row_face(*r);
                (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
            })
            .collect();
        let hint = shortcut_hint_face();
        let hint = (
            compositor.create_texture(hint.w, hint.h, &hint.rgba),
            hint.w,
            hint.h,
        );
        self.session.app_mut().set_shortcut_faces(rows, Some(hint));
    }

    /// The mode moved: re-rasterise everything whose ink is burned in, and drop the caches of
    /// everything that will be rasterised again on demand.
    ///
    /// Two halves, and both are needed. `upload_fixed` covers the faces that only exist once —
    /// the glyphs, the menus, the letters, the shortcut card. The caches below cover the faces
    /// that are built lazily as the screens need them: the shelf's title, the clock and the
    /// charge on the band, the about card, the switcher's caption. Clearing them is what makes
    /// the *next* frame rebuild them rather than the last one; the four defaults are the value
    /// each compares against to decide it has nothing worth keeping.
    ///
    /// The cart faces are deliberately not in either half. A label is a picture and a shell is a
    /// colour off the game's code: nothing about a cartridge is printed in the palette, which is
    /// the same reason `upload_faces` can put nine hundred of them outside the boot's critical
    /// path.

    /// Rasterise the fixed furniture again, in whatever mode is now set, and rebuild the cart
    /// faces the row is drawing.
    ///
    /// The whole reason a theme change does not need a restart: the type is a texture, so the
    /// ink cannot move under it, so the textures have to be made again. What it deliberately
    /// does *not* touch is the font and the labels — a label is a picture and has no palette in
    /// it, and asking for nine hundred of them again would be a second of boot budget spent on
    /// colours they never had.
    fn rebake(&mut self, compositor: &mut Compositor) {
        let at = Instant::now();
        self.upload_fixed(compositor);
        // The row's frame bakes a palette colour too — the blank cart is the housing's on the
        // light theme — so a theme change re-cuts it exactly as a machine swap does. Without
        // this the stand-in kept the other theme's colour until something else rebuilt it, and
        // a letter jump is mostly stand-ins: that is where it showed.
        self.rebake_row_frame(compositor);
        self.title_tex = None;
        self.shelf_title_tex = None;
        self.shelf_titled = None;
        self.switcher = Switcher::default();
        self.clocks = Clocks::default();
        self.about = AboutFace::default();
        // Cart faces: the visible window is rebuilt in place; everything outside it is released
        // and cleared so the resident pump rebuilds it on demand. `resident_at = None` forces
        // `sync_resident_faces` to rescan on the next frame and re-queue the dropped faces.
        let count = self.session.app().carts().len();
        let index = self.session.app().shelf_index();
        let window = self.session.app().shelf_draw_window();
        let window = if window.is_empty() {
            shelf_window(index, count)
        } else {
            window
        };
        for i in 0..count {
            if window.contains(&i) {
                let f = cart_face(&self.session.app().carts()[i]);
                let tex = compositor.create_texture(f.w, f.h, &f.rgba);
                self.session.app_mut().set_face(i, tex);
            } else if let Some(tex) = self.session.app().face_of(i) {
                compositor.release_texture(tex);
                self.session.app_mut().clear_face(i);
            }
        }
        self.resident_at = None;
        self.session.boot_note(&format!(
            "reprinted for the mode in {} ms",
            at.elapsed().as_millis()
        ));
    }

    /// A machine shelf swapped in mid-session: re-cut the row's frame for the machine that is up
    /// and build the carts the new row draws.
    ///
    /// Three things were minted for the shelf the device came up on and are wrong the moment
    /// another one is swapped in: the shadow under a dimmed cart, the blank cart a not-yet-built
    /// face stands in as (an Advance cart is landscape and a Game Boy cart portrait, so the
    /// stand-in is visibly the wrong object), and the set of faces worth having. All three are
    /// rebuilt here, in the one place with a compositor.
    ///
    /// The faces are the reason this exists rather than being cosmetic. A swap changes the view
    /// to a *subset* of the library, and the filler orders its work by ring distance in the
    /// library — so the carts on screen are not necessarily the ones it would reach first, and
    /// until it gets there the row shows the stand-in. Building the seven the row draws, here,
    /// means the shelf is right on the frame it appears rather than at some point after.
    fn set_shelf_geometry(&mut self, compositor: &mut Compositor) {
        let at = Instant::now();
        let sys = self.session.app().shelf_system();
        self.rebake_row_frame(compositor);
        // The faces need nothing here, and that is the point of measuring in the shelf rather
        // than in the library. `Shelf::shelf_distance` counts from the front of *every*
        // machine's shelf as well as from the live caret, so the row this swap is about to show
        // has been resident the whole time. Measuring in the library evicted it as "too far
        // away", and rebuilding it here cost 129-179 ms on the render thread, in the middle of
        // a 0.68 s swap animation. Nothing to build now, so nothing to stall on.
        let count = self.session.app().carts().len();
        // A rescan is still wanted, so the queue matches the new view rather than the old one.
        self.resident_at = None;
        // The row, as the draw sees it: which carts are in the slots, and which of those are
        // holding a face rather than the stand-in. A slot with `N` here is the stand-in on
        // screen, and a slot index that is not in `window` means the two are answering
        // different questions.
        let slots: Vec<usize> = self.session.app().shelf_slot_carts();
        let marks: String = slots
            .iter()
            .map(|&i| {
                if self.session.app().face_of(i).is_some() {
                    'Y'
                } else {
                    'N'
                }
            })
            .collect();
        self.session.boot_note(&format!(
            "shelf {} lib {} view {} idx {} slots {:?} faces {}",
            sys.dir_name(),
            count,
            self.session.app().shelf_view_len(),
            self.session.app().shelf_index(),
            slots,
            marks
        ));
        self.session.boot_note(&format!(
            "shelf geometry {} in {} ms",
            sys.dir_name(),
            at.elapsed().as_millis()
        ));
    }

    /// The row's own frame — the shadow under a dimmed cart, and the blank cart a not-yet-built
    /// face stands in as. Both bake a colour the palette owns: the light theme repaints the
    /// built-in shell in the housing's colour (`cart::themed_shell`), so the blank cart is a
    /// texture that goes stale the instant the mode flips. That is why this is one function —
    /// `rebake` (theme) and `set_shelf_geometry` (machine) are two different reasons to want
    /// the same two textures minted, and only the first knew about it before.
    fn rebake_row_frame(&mut self, compositor: &mut Compositor) {
        let sys = self.session.app().shelf_system();
        let size = size_for(sys);
        let shadow = cart_shadow(size, sys);
        let id = compositor.create_texture(shadow.w, shadow.h, &shadow.rgba);
        if let Some(old) = self.session.app_mut().set_cart_shadow(id) {
            compositor.release_texture(old);
        }
        let blank = cart_placeholder(size, sys);
        let blank = compositor.create_texture(blank.w, blank.h, &blank.rgba);
        if let Some(old) = self.session.app_mut().set_cart_placeholder(blank) {
            compositor.release_texture(old);
        }
    }

    /// The third boot line: how long the card's faces took, which is the only part of the boot
    /// that grows with the library. Split out so the timing call does not need a borrow of the
    /// compositor that `upload_faces` has already given back.
    fn upload_faces_note(&self, since: Instant) {
        self.session.boot_note(&format!(
            "shelf faces total {} ms",
            since.elapsed().as_millis()
        ));
        // Where that time went, by pass. A hundred-game card and a seventy-millisecond face
        // is a number worth splitting before anyone decides what to optimise.
        self.session.boot_note(&format!(
            "cart face phases: {}",
            slot_ui::face_profile::line()
        ));
    }

    fn poll_ui_faces(&mut self, compositor: &mut Compositor) {
        let Some(rx) = &self.ui_faces else {
            return;
        };
        match rx.try_recv() {
            Ok(faces) => {
                self.ui_faces = None;
                self.place_ui_faces(compositor, faces);
                crate::boot_time::mark("ui faces");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.ui_faces = None,
        }
    }

    fn place_ui_faces(&mut self, compositor: &mut Compositor, faces: UiFaces) {
        let mut cart = |f: CartFace| compositor.create_texture(f.w, f.h, &f.rgba);
        let link_badges = faces.link_badges.into_iter().map(&mut cart).collect();
        let toasts = faces.toasts.into_iter().map(&mut cart).collect();
        let sockets = faces.sockets.into_iter().map(&mut cart).collect();
        let chips = faces.chips.into_iter().map(&mut cart).collect();
        let blank = cart(faces.blank_chip);
        let shadow = cart(faces.chip_shadow);
        let mut up = |f: UndoFace| (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h);
        let lines = faces.shutdown.into_iter().map(&mut up).collect();
        let menu = faces.power_menu.into_iter().map(&mut up).collect();
        let labels = faces.quick_labels.into_iter().map(&mut up).collect();
        let values = faces
            .quick_values
            .into_iter()
            .map(|v| v.map(&mut up))
            .collect();
        let carets = faces.quick_carets.map(&mut up);
        let legend = faces.quick_legend.map(|f| {
            let (tex, w, _) = up(f);
            (tex, w)
        });
        let core_legend = faces
            .core_legend
            .into_iter()
            .map(|f| {
                let (tex, w, _) = up(f);
                (tex, w)
            })
            .collect();
        let roles = faces.link_roles.into_iter().map(&mut up).collect();
        let linked = up(faces.linked);
        let link_legend = faces
            .link_legend
            .into_iter()
            .map(|f| {
                let (tex, w, _) = up(f);
                (tex, w)
            })
            .collect();
        let steps = faces.link_steps.into_iter().map(&mut up).collect();
        let fails = faces.link_fails.into_iter().map(&mut up).collect();
        let app = self.session.app_mut();
        app.set_link_badge_faces(link_badges);
        app.set_shutdown_faces(lines);
        app.set_power_menu_faces(menu);
        app.set_quick_menu_faces(QuickMenuFaces {
            labels,
            values,
            carets,
            legend,
        });
        app.set_core_part_faces(sockets, chips, blank, shadow);
        app.set_core_legend_faces(core_legend);
        app.set_link_menu_faces(roles);
        app.set_link_linked_face(linked);
        app.set_link_legend_faces(link_legend);
        app.set_link_step_faces(steps);
        app.set_link_fail_faces(fails);
        app.set_toast_faces(toasts);
    }

    fn upload_wallpaper(&mut self, compositor: &mut Compositor) {
        let app = self.session.app();
        let seed = app.wall_secs().unsigned_abs();
        let Some(rgba) = app
            .root()
            .and_then(|root| wallpaper::pick(root, seed))
            .and_then(|path| wallpaper_face(&path))
        else {
            return;
        };
        let id = compositor.create_texture(OUT_W, OUT_H, &rgba);
        self.session.app_mut().set_wallpaper(id);
    }

    /// Upload whatever the shelf filler finished, then ask it for the cart nearest the caret
    /// that is still missing a face.
    ///
    /// One request in flight at a time: the answer is always wanted, and a queue would only
    /// let the worker fall further behind where the caret actually is.
    fn pump_shelf_faces(&mut self, compositor: &mut Compositor) {
        self.sync_resident_faces(compositor);
        for (i, f) in self.shelf_faces.drain() {
            let id = compositor.create_texture(f.w, f.h, &f.rgba);
            self.session.app_mut().set_face(i, id);
        }
        if self.shelf_faces.busy() || self.shelf_missing.is_empty() {
            return;
        }
        // Only within the resident window, and the window is the row's own order — the carts
        // the caret could reach by stepping, not by index arithmetic. Without the cap the
        // filler would keep working outward and the cap would be a treadmill; without the view
        // it works outward through carts no shelf is showing.
        let Some(pos) = self
            .shelf_missing
            .iter()
            .enumerate()
            .filter_map(|(p, &i)| {
                self.session
                    .app()
                    .shelf_distance(i)
                    .filter(|&d| d <= RESIDENT)
                    .map(|d| (d, p))
            })
            .min_by_key(|&(d, _)| d)
            .map(|(_, p)| p)
        else {
            return;
        };
        let i = self.shelf_missing.swap_remove(pos);
        let cart = self.session.app().carts()[i].clone();
        self.shelf_faces.request(i, cart);
    }

    /// Keeps the textures resident to a window around the caret, and the queue of carts still
    /// wanting one.
    ///
    /// A face is `FACE_W * FACE_H * 4` — a little over half a megabyte — and costs about 26 ms
    /// to rasterise, so a card of nine hundred games would hold nearly five hundred megabytes
    /// of shelf if every cart kept its own. The shelf draws three either side; this keeps
    /// twelve, releases what falls outside, and re-queues it for the build it will need if the
    /// caret ever comes back.
    ///
    /// Runs on the caret's edge rather than every frame: it walks the whole library, and the
    /// caret moves at most a few times a second.
    fn sync_resident_faces(&mut self, compositor: &mut Compositor) {
        let count = self.session.app().carts().len();
        if count == 0 {
            return;
        }
        let index = self.session.app().shelf_index();
        if self.resident_at == Some(index) {
            return;
        }
        self.resident_at = Some(index);

        self.shelf_missing.clear();
        let mut released = Vec::new();
        for i in 0..count {
            // Kept by *view* distance: see `Shelf::view_distance`. A cart the shelf up does not
            // hold is not worth a face at all, and one it does hold is worth one however far
            // away it sits in the library.
            let held = self
                .session
                .app()
                .shelf_distance(i)
                .is_some_and(|d| d <= RESIDENT + RESIDENT_HYSTERESIS);
            match (self.session.app().face_of(i), held) {
                (Some(tex), false) => released.push((i, tex)),
                (None, _) => self.shelf_missing.push(i),
                _ => {}
            }
        }
        for (i, tex) in released {
            compositor.release_texture(tex);
            self.session.app_mut().clear_face(i);
            self.shelf_missing.push(i);
        }
    }

    pub fn upload_bezel(&mut self, compositor: &mut Compositor, panel: (u32, u32)) {
        if !slot_gfx::framed(panel) {
            return;
        }
        slot_ui::set_shelf_slack((OUT_W * panel.1 / panel.0).saturating_sub(OUT_H) as f32);
        let Some(path) = self
            .session
            .app()
            .root()
            .and_then(|root| crate::bezel::pick(root, panel))
        else {
            return;
        };
        if let Some(rgba) = bezel_face(&path, panel.0, panel.1) {
            compositor.set_bezel(panel.0, panel.1, &rgba);
            eprintln!("slot: bezel {}", path.display());
        }
    }

    pub fn render(&mut self, compositor: &mut Compositor, window: (u32, u32)) {
        compositor.fit(window);
        self.compose(compositor);
        compositor.end_frame(window);
    }

    /// One frame into the offscreen target and out to a surface of `window` pixels.
    pub fn compose(&mut self, compositor: &mut Compositor) {
        // The one number that can be compared across base OSes, logged once on the frame the
        // user first sees. Everything else in `boot.log` is a duration, and a duration cannot
        // tell you which base you booted — nor does it carry what the kernel and initramfs cost
        // before this process started, which is exactly what a faster base trims.
        //
        // Device uptime is the right clock: it starts at kernel boot, so it includes the kernel
        // and initramfs, and the bootloader before it is the same whichever base is underneath.
        // The base's own version comes with it so the log says what it was measured on.
        if !self.frame_logged {
            self.frame_logged = true;
            let uptime = std::fs::read_to_string("/proc/uptime")
                .ok()
                .and_then(|s| s.split_whitespace().next().map(str::to_string))
                .unwrap_or_else(|| "?".into());
            let base = std::fs::read_to_string("/etc/os-release")
                .ok()
                .and_then(|s| {
                    s.lines().find(|l| l.starts_with("VERSION_ID=")).map(|l| {
                        l["VERSION_ID=".len()..]
                            .trim()
                            .trim_matches('"')
                            .to_string()
                    })
                })
                .unwrap_or_else(|| "unknown".into());
            self.session.boot_note(&format!(
                "first frame at {} s of uptime  (base VERSION_ID={})",
                uptime, base
            ));
        }
        // The mode, before the frame is built: it is the one thing the app can change that the
        // textures cannot follow on their own, so the app raises a flag and this is what takes
        // it. Done here rather than in `advance` because this is the only place with a
        // compositor, and a texture cannot be minted without one.
        if self.session.app_mut().take_mode_dirty() {
            self.rebake(compositor);
        }
        // The other thing the app can change that a texture cannot follow on its own: which
        // machine's shelf is up. Same reason, same place.
        if self.session.app_mut().take_shelf_dirty() {
            self.set_shelf_geometry(compositor);
        }
        // First, so a face that finished since the last frame is on screen this frame.
        self.pump_shelf_faces(compositor);
        if !self.row_logged {
            self.row_logged = true;
            let (sys, lib, view, idx, slots, marks, win) = {
                let app = self.session.app();
                let slots = app.shelf_slot_carts();
                let marks: String = slots
                    .iter()
                    .map(|&i| if app.face_of(i).is_some() { 'Y' } else { 'N' })
                    .collect();
                (
                    app.shelf_system().dir_name().to_string(),
                    app.carts().len(),
                    app.shelf_view_len(),
                    app.shelf_index(),
                    slots,
                    marks,
                    app.shelf_draw_window(),
                )
            };
            self.session.boot_note(&format!(
                "shelf row {} lib {} view {} idx {} slots {:?} faces {} window {:?} cc {}",
                sys,
                lib,
                view,
                idx,
                slots,
                marks,
                win,
                self.session.app().display_palette_name()
            ));
        }
        // Set every frame rather than on the edge: the grade is part of the final blit, so
        // it has to be right whether or not anything just changed it.
        compositor.set_blue_light(self.session.app().blue_light());
        compositor.set_picture(self.session.app().picture_rect());
        compositor.set_screen_effect(match self.session.app().screen_shader() {
            slot_store::Shader::Off => ScreenEffect::None,
            slot_store::Shader::Lcd3x => ScreenEffect::Lcd3x,
            slot_store::Shader::Grid => ScreenEffect::Grid,
            slot_store::Shader::Dot => ScreenEffect::Dot,
            slot_store::Shader::Simpletex => ScreenEffect::Simpletex,
        });
        compositor.set_shake(self.session.app().screen_shake());
        compositor.set_screen_power(self.session.app().screen_power());
        // The card's display filter, every frame so a SELECT+X (that machine's own screen look)
        // or SELECT+Y (colour) press lands on the next one.
        compositor.set_panel_mask(&self.session.app().display_mask());
        compositor.set_color_correction(
            &self.session.app().display_cc(),
            &self.session.app().display_cc_bias(),
        );
        // The Game Boy's colour, which is a palette lookup rather than a matrix: four shades,
        // four colours, and the table does the rest. `None` on every other machine and on
        // 原生灰, which leaves the matrix path — and its two `pow`s — switched off.
        let palette = self.session.app().display_palette();
        compositor.set_palette(palette.as_ref());
        // The pixel-art lookup, the other texture-driven grade: `None` unless the pixel mode is
        // the one in force on a colour machine, so it costs nothing the rest of the time.
        compositor.set_pixel_lut(self.session.app().display_pixel_lut());
        // ...and the lattice over it, on the two machines that draw one: a Game Boy's is the
        // palette's own lightest shade and a Game Boy Color's is white or black and rides the
        // overlay ring. An Advance has no overlay, so it is on the aperture table instead
        // (`display_mask`) and pushes nothing here.
        match self.session.app().display_grid() {
            Some((colour, mix, scanline)) => compositor.set_grid(&colour, mix, scanline),
            None => compositor.set_grid(&[0.0, 0.0, 0.0], 0.0, false),
        }
        compositor.set_cc_gamma(self.session.app().display_cc_gamma());
        // Every frame, like the rest: the machine whose frame these filters are about is the
        // cart under the highlight, and walking the row changes it. Cheap — the pass keeps its
        // own answer and does nothing at all until it differs.
        compositor.set_system(self.session.app().screen_system());
        compositor.set_frame_lift(self.session.app().frame_lift());
        compositor.set_frame_split(self.session.app().frame_split());
        compositor.set_game_source_rect(self.session.app().source_rect());
        compositor.begin_frame();
        if self.draw_rebuild(compositor) {
            return;
        }
        if let Some(frame) = self.session.frame() {
            compositor.upload_game(&frame);
            crate::latency::taken();
        }
        self.cart_faces.sync(self.session.app_mut(), compositor);
        sync_clock(self.session.app_mut(), compositor, &mut self.clocks);
        sync_about(self.session.app_mut(), compositor, &mut self.about);
        sync_shelf_title(
            self.session.app_mut(),
            compositor,
            &mut self.shelf_title_tex,
            &mut self.shelf_titled,
        );
        sync_quick_clock(self.session.app_mut(), compositor, &mut self.quick_clock);
        sync_core_picker(
            self.session.app_mut(),
            compositor,
            &self.faces,
            &mut self.core_asked,
            &mut self.core_board_tex,
            &mut self.core_lid_tex,
            &mut self.core_built,
        );
        if !self.link_art_done {
            if let Some(art) = self.link_art.take() {
                let mut up = |f: &slot_ui::CartFace| Sprite {
                    tex: compositor.create_texture(f.w, f.h, &f.rgba),
                    w: f.w,
                    h: f.h,
                };
                let sprites = LinkSprites {
                    port: up(&art.port),
                    plug_host: up(&art.plug_host),
                    plug_join: up(&art.plug_join),
                    adapter: up(&art.adapter),
                    arcs_right: [
                        up(&art.arcs_right[0]),
                        up(&art.arcs_right[1]),
                        up(&art.arcs_right[2]),
                    ],
                    arcs_left: [
                        up(&art.arcs_left[0]),
                        up(&art.arcs_left[1]),
                        up(&art.arcs_left[2]),
                    ],
                    clicks: up(&art.clicks),
                    arrow_left: up(&art.arrow_left),
                    arrow_right: up(&art.arrow_right),
                };
                self.session.app_mut().set_link_sprites(sprites);
                self.link_art_done = true;
            }
        }
        sync_switcher(
            self.session.app_mut(),
            compositor,
            Faces {
                pool: &mut self.polaroid_texes,
                title: &mut self.title_tex,
                undo: &mut self.undo_tex,
            },
            &mut self.switcher,
        );
        sync_cheat_faces(self.session.app_mut(), compositor);
        sync_palette_names(
            self.session.app_mut(),
            compositor,
            &mut self.palette_texes,
            &mut self.palette_paged,
        );
        sync_palette_name(
            self.session.app_mut(),
            compositor,
            &mut self.palette_tex,
            &mut self.palette_named,
        );
        sync_overlay(self.session.app_mut(), compositor);
        // The screen reflection: handed the overlay texture as its zone mask when the overlay
        // up is a reflective one, `None` otherwise. Set every frame with the rest of the state.
        compositor.set_reflection(if self.session.app().overlay_reflects() {
            self.session.app().overlay_tex()
        } else {
            None
        });
        self.draws.clear();
        self.session.app().draw(&mut self.draws);
        compositor.draw_list(&self.draws);
    }

    pub fn drive_emulator(&mut self) {
        self.session.set_driven(true);
    }

    pub fn step_emulator(&self, present: Duration, timeout: Duration) -> bool {
        self.session.step_emulator(present, timeout)
    }

    pub fn rebuilding_labels(&self) -> bool {
        self.rebuild.is_some()
    }

    fn check_labels(
        root: &std::path::Path,
        carts: Vec<slot_store::Cart>,
    ) -> Option<std::sync::mpsc::Receiver<Vec<slot_store::Cart>>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let root = root.to_path_buf();
        std::thread::Builder::new()
            .name("slot-label-check".into())
            .spawn(move || {
                let _ = tx.send(label_cache::stale(&root, carts.iter()));
            })
            .ok()
            .map(|_| rx)
    }

    fn poll_label_check(&mut self) {
        let Some(rx) = &self.stale_check else {
            return;
        };
        let stale = match rx.try_recv() {
            Ok(stale) => stale,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.stale_check = None;
        let Some(root) = self.session.app().root().map(|r| r.to_path_buf()) else {
            return;
        };
        if !stale.is_empty() {
            eprintln!("slot: label cache: {} new labels", stale.len());
            self.rebuild = Some(Rebuild::start(&root, stale));
        }
    }

    fn draw_rebuild(&mut self, compositor: &mut Compositor) -> bool {
        self.poll_label_check();
        self.poll_ui_faces(compositor);
        let Some(rebuild) = &self.rebuild else {
            return false;
        };
        if rebuild.finished() {
            self.rebuild = None;
            self.last = Instant::now();
            return false;
        }
        let faces = &mut self.rebuild_faces;
        let title = *faces.title.get_or_insert_with(|| {
            let f = rebuild_title_face("Caching New Labels");
            (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
        });
        let done = rebuild.done();
        if faces.shown != Some(done) {
            let f = rebuild_count_face(&format!("{done} / {}", rebuild.total()));
            faces.count_size = (f.w, f.h);
            upload(compositor, &mut faces.count, f);
            faces.shown = Some(done);
        }
        let screen = RebuildScreen {
            title: Some(title),
            count: faces
                .count
                .map(|tex| (tex, faces.count_size.0, faces.count_size.1)),
            fraction: done as f32 / rebuild.total().max(1) as f32,
        };
        self.draws.clear();
        screen.draw(&mut self.draws);
        compositor.draw_list(&self.draws);
        true
    }

    pub fn advance(&mut self, input: &mut dyn InputSource) {
        let now = self.now();
        let events = input.poll(now);
        if self.rebuild.is_some() {
            return;
        }
        self.session.feed(events, now);
        let dt = self.last.elapsed().as_secs_f32();
        self.last = Instant::now();
        self.session.update(dt);
    }

    pub fn app(&self) -> &crate::app::App {
        self.session.app()
    }

    pub fn core_settling(&self) -> bool {
        self.session.core_settling()
    }

    pub fn advance_at(&mut self, input: &mut dyn InputSource, now: Millis, dt: f32) {
        let events = input.poll(now);
        self.session.feed(events, now);
        self.session.update(dt);
    }

    fn now(&self) -> Millis {
        self.start.elapsed().as_millis() as Millis
    }

    pub fn powering_off(&self) -> bool {
        self.session.app().ready_to_power_off()
    }

    pub fn restarting(&self) -> bool {
        self.session.app().ready_to_restart()
    }

    pub fn restart(&mut self) {
        self.session.app_mut().restart();
    }

    pub fn poweroff(&mut self) {
        self.session.app_mut().poweroff();
    }
}

fn menu_faces(
    compositor: &mut Compositor,
    lines: impl Iterator<Item = impl AsRef<str>>,
) -> Vec<(TexId, u32, u32)> {
    lines
        .map(|line| {
            let f = menu_face(line.as_ref());
            (
                compositor.create_texture(f.w, f.h, &f.rgba),
                f.w,
                f.h,
            )
        })
        .collect()
}

fn legend_faces(compositor: &mut Compositor, legend: &[(&str, &str)]) -> Vec<TexId> {
    legend
        .iter()
        .map(|(key, label)| {
            let f = hint_face(key, label);
            compositor.create_texture(f.w, f.h, &f.rgba)
        })
        .collect()
}

struct UiFaces {
    link_badges: Vec<CartFace>,
    shutdown: Vec<UndoFace>,
    power_menu: Vec<UndoFace>,
    quick_labels: Vec<UndoFace>,
    quick_values: Vec<[UndoFace; 2]>,
    quick_carets: [UndoFace; 2],
    quick_legend: [UndoFace; 3],
    sockets: Vec<CartFace>,
    chips: Vec<CartFace>,
    blank_chip: CartFace,
    chip_shadow: CartFace,
    core_legend: Vec<UndoFace>,
    link_roles: Vec<UndoFace>,
    linked: UndoFace,
    link_legend: Vec<UndoFace>,
    link_steps: Vec<UndoFace>,
    link_fails: Vec<UndoFace>,
    toasts: Vec<CartFace>,
}

impl UiFaces {
    fn build() -> Self {
        UiFaces {
            link_badges: LinkBadge::FACES
                .iter()
                .map(|b| {
                    let (badge, ink) = (
                        b.badge().expect("a face has a glyph"),
                        b.colour().expect("and a colour"),
                    );
                    badge_face(badge, HUD_ICON_PX, ink)
                })
                .collect(),
            shutdown: PowerChoice::ALL
                .iter()
                .map(|c| {
                    menu_face(match c {
                        PowerChoice::Restart => "Restarting",
                        PowerChoice::PowerOff => "Powering Down",
                    })
                })
                .collect(),
            power_menu: PowerChoice::ALL
                .iter()
                .map(|c| menu_face(c.text()))
                .collect(),
            quick_labels: QuickRow::ALL.iter().map(|r| quick_label_face(*r)).collect(),
            quick_values: QuickValue::ALL
                .iter()
                .map(|v| [false, true].map(|lit| quick_value_face(v.text(), lit)))
                .collect(),
            quick_carets: [false, true].map(quick_caret_face),
            quick_legend: quick_legend_faces(),
            sockets: slot_store::Core::ALL
                .iter()
                .map(|c| socket_face(*c))
                .collect(),
            chips: slot_store::Core::ALL
                .iter()
                .map(|c| chip_face(Some(*c)))
                .collect(),
            blank_chip: chip_face(None),
            chip_shadow: chip_shadow_face(),
            core_legend: vec![
                hint_face("B", "Cancel"),
                arrows_hint_face("Swap"),
                hint_face("A", "Choose"),
            ],
            link_roles: LinkRow::ALL.iter().map(|r| menu_face(r.text())).collect(),
            linked: menu_face("Linked"),
            link_legend: vec![
                hint_face("B", "Cancel"),
                hint_face("SELECT", "Mode"),
                arrows_hint_face("Swap"),
                hint_face("A", "Link"),
                hint_face("A", "OK"),
                hint_face("B", "Back"),
                hint_face("A", "End Link"),
            ],
            link_steps: LinkStep::ALL.iter().map(|s| menu_face(s.line())).collect(),
            link_fails: LinkFail::SHOWN
                .iter()
                .map(|f| menu_face(f.line()))
                .collect(),
            toasts: Toast::all().into_iter().map(toast_face).collect(),
        }
    }
}

struct Faces<'a> {
    pool: &'a mut Vec<TexId>,
    title: &'a mut Option<TexId>,
    undo: &'a mut Option<TexId>,
}

fn sync_switcher(app: &mut App, compositor: &mut Compositor, texes: Faces, state: &mut Switcher) {
    if !matches!(app.phase(), Phase::Polaroids { .. }) {
        state.open = false;
        return;
    }
    if !state.open {
        state.open = true;
        state.titled = None;
        let faces: Vec<_> = app.polaroid_entries().iter().map(photo_face).collect();
        let ids = faces
            .iter()
            .enumerate()
            .map(|(i, f)| match texes.pool.get(i) {
                Some(id) => {
                    compositor.update_texture(*id, f.w, f.h, &f.rgba);
                    *id
                }
                None => {
                    let id = compositor.create_texture_nearest(f.w, f.h, &f.rgba);
                    texes.pool.push(id);
                    id
                }
            })
            .collect();
        app.set_polaroid_faces(ids);

        let label = app
            .undo_label()
            .map(|l| upload(compositor, texes.undo, hint_face("X", l)));
        app.set_undo_face(label);
    }
    if state.titled.as_deref() != app.polaroid_stamp() {
        state.titled = app.polaroid_stamp().map(str::to_string);
        let face = title_face(&app.polaroid_title(&format_stamp(app.wall_secs())));
        let id = upload(compositor, texes.title, face);
        app.set_polaroid_title_face(id);
    }
}

fn sync_clock(app: &mut App, compositor: &mut Compositor, clocks: &mut Clocks) {
    let picked = app.picker().map(|p| p.text());
    if picked != clocks.picked {
        clocks.picked = picked;
        if let (Some(face), Some(hint)) = (app.picker().map(|p| p.face()), clocks.hint) {
            let line = upload(compositor, &mut clocks.line, face);
            app.set_clock_faces(line, hint);
        }
    }
    let shown = hhmm(app.wall_secs());
    if shown != clocks.shown {
        // The clock and the battery's percent are the two halves of one status row and are set at
        // the same size now, in the same `STATUS_H`-tall band — `clock_face` and `word_face` own
        // those two numbers, so nothing here has to say what they are.
        let face = clock_face(&shown);
        clocks.shown = shown;
        let w = face.w;
        let id = upload(compositor, &mut clocks.shelf, face);
        app.set_shelf_clock_face(id, w);
    }
    let platform_shown = app.slot_text().unwrap_or_default();
    if platform_shown != clocks.platform {
        clocks.platform = platform_shown.clone();
        if platform_shown.is_empty() {
            app.clear_shelf_platform();
        } else {
            let face = word_face(&platform_shown);
            let w = face.w;
            let id = upload(compositor, &mut clocks.platform_tex, face);
            app.set_shelf_platform_face(id, w);
        }
    }
    let battery_shown = app
        .battery()
        .map(|b| format!("{}%", b.percent))
        .unwrap_or_default();
    if battery_shown != clocks.battery {
        clocks.battery = battery_shown.clone();
        if !battery_shown.is_empty() {
            let face = word_face(&battery_shown);
            let w = face.w;
            let id = upload(compositor, &mut clocks.battery_tex, face);
            app.set_battery_percent_face(id, w);
        }
    }
    // The direct-connect address: baked when the connect script has left one behind, because DHCP
    // decides it and the dialog's baked-at-boot lines cannot carry it.
    if app.direct_ip_dirty() {
        match app.direct_ip_str().map(str::to_string) {
            Some(ip) if !ip.is_empty() => {
                let face = slot_ui::dialog_line_face(&ip, 24.0);
                let (w, h) = (face.w, face.h);
                let id = compositor.create_texture(w, h, &face.rgba);
                app.set_direct_ip_face(Some((id, w, h)));
            }
            _ => app.set_direct_ip_face(None),
        }
        app.clear_direct_ip_dirty();
    }
}

fn sync_quick_clock(app: &mut App, compositor: &mut Compositor, state: &mut QuickClock) {
    if app.quick_menu().is_none() {
        return;
    }
    let text = date_time_text(app.wall_secs());
    if text == state.shown {
        return;
    }
    let (dim, lit) = (
        quick_value_face(&text, false),
        quick_value_face(&text, true),
    );
    let (dim_size, lit_size) = ((dim.w, dim.h), (lit.w, lit.h));
    let dim = upload(compositor, &mut state.dim, dim);
    let lit = upload(compositor, &mut state.lit, lit);
    app.set_quick_clock_faces((dim, dim_size.0, dim_size.1), (lit, lit_size.0, lit_size.1));
    state.shown = text;
}

fn sync_about(app: &mut App, compositor: &mut Compositor, state: &mut AboutFace) {
    if !matches!(app.phase(), Phase::About { .. }) {
        return;
    }
    let battery = app.battery().map(|b| b.percent);
    if state.tex.is_some() && state.battery == battery {
        return;
    }
    state.battery = battery;
    let build = Build::current();
    let face = sticker_face(&StickerFields {
        battery,
        serial: &build.serial(),
        dirty_digit: build.dirty_digit(),
    });
    let id = upload(compositor, &mut state.tex, face);
    app.set_sticker_face(id);
}

/// The open cart's faces, asked for as soon as the caret lands on a cart and uploaded when the
/// worker hands them back, so they are normally on the GPU before START. The worker is the only
/// place they are built: rasterised on the frame loop, a board freezes the shelf for the better
/// part of half a second on the H700.
/// The game under the eye on the shelf, rebuilt only when the cart under the eye changes.
/// One rasterise per arrow press, and none at all on a frame where nothing moved.
fn sync_shelf_title(
    app: &mut App,
    compositor: &mut Compositor,
    slot: &mut Option<TexId>,
    shown: &mut Option<String>,
) {
    let want = app.selected_stem().map(str::to_string);
    if *shown == want {
        return;
    }
    *shown = want.clone();
    match want {
        Some(stem) => {
            // Named from the cart's own name, cut out of the filename the same way its printed
            // label is, so the two readings cannot disagree about what the game is called.
            let face = shelf_title_face(&clean_label(&stem));
            let (w, h) = (face.w, face.h);
            let id = upload(compositor, slot, face);
            app.set_shelf_title_face(Some((id, w, h)));
        }
        None => app.set_shelf_title_face(None),
    }
}

/// The palette browser's row of names: one face per cell, so every cell on the panel says what
/// it is rather than only the one under the caret. A colour machine's page, whose nine grades are
/// named in four to six characters each.
///
/// Mirrors `sync_shelf_title` in what it costs and how: the key is the page, so a turn of the
/// page is the only thing here that rasterises anything, and a caret walking inside a page —
/// which is nearly every frame the browser is up — is a comparison and nothing else. The
/// textures come from a pool and go back into it, and every face is the cell's own width, so
/// the pool neither grows nor has to be resized after the first page.
///
/// Empty on a Game Boy, whose names do not fit a cell: the faces are cleared and
/// `sync_palette_name` sets the foot line instead.
fn sync_palette_names(
    app: &mut App,
    compositor: &mut Compositor,
    pool: &mut Vec<Option<TexId>>,
    shown: &mut Option<(System, usize)>,
) {
    let key = app.palette_names_key();
    if *shown == key {
        return;
    }
    *shown = key;
    if key.is_none() {
        app.set_palette_names(Vec::new());
        return;
    }
    let (w, h) = (palette_cell_w(), palette_name_h());
    let names = app.palette_names();
    let mut faces = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        // The author's own name, number and all: `PS40 Sunburst` is what the same palette is
        // called in `gbcpalettes.h`, so a name read off the device can be looked up there
        // without a translation table in between. Fixed-size boxes, so a long one is set smaller
        // rather than clipped and the pool below is reused instead of rebuilt.
        let face = palette_name_face(name, w, h);
        while pool.len() <= i {
            pool.push(None);
        }
        let id = upload_rgba(compositor, &mut pool[i], w, h, &face.rgba);
        faces.push((id, w, h));
    }
    app.set_palette_names(faces);
}

/// The palette browser's one line of type: the name of the entry under the caret, for the tables
/// whose names are longer than a cell. Mirrors `sync_shelf_title` — one rasterise per move of the
/// caret, and none at all on a frame where nothing moved.
///
/// The box is `dialog_line_face`'s, 640 pixels wide and twice the type size tall, so a name that
/// no cell could hold is set here whole, at the size the rest of the interface uses.
fn sync_palette_name(
    app: &mut App,
    compositor: &mut Compositor,
    slot: &mut Option<TexId>,
    shown: &mut Option<String>,
) {
    let want = app.palette_name_want();
    if *shown == want {
        return;
    }
    *shown = want.clone();
    match want {
        Some(name) => {
            let face = dialog_line_face(&name, PALETTE_FOOT_PX);
            let (w, h) = (face.w, face.h);
            let id = upload(compositor, slot, face);
            app.set_palette_name_face(Some((id, w, h)));
        }
        None => app.set_palette_name_face(None),
    }
}

/// (Re)build the cheat table's row faces when the seated cart changes. Mirrors `sync_shelf_title`:
/// only rasterises when the highlight's stem differs from the one the faces were last built for,
/// so a long list is rasterised once per cart, not once per frame.
fn sync_cheat_faces(app: &mut App, compositor: &mut Compositor) {
    let want = app.selected_stem().map(str::to_string);
    let n = app.cheats().len();
    // Rebuild when the cart changes OR when the code count changes. Cheats load from the card
    // after the first build (when the core spawns), so an early build would otherwise cache empty
    // faces and the table would show chips with no text.
    if app.cheat_face_stem() == want.as_deref() && app.cheat_face_count() == n {
        return;
    }
    match want {
        Some(stem) => {
            let rows: Vec<(String, String)> = app
                .cheats()
                .iter()
                .map(|c| (c.desc.clone(), c.code.clone()))
                .collect();
            let faces: Vec<(TexId, u32, u32)> = rows
                .iter()
                .map(|(desc, code)| {
                    let f = cheat_row_face(desc, code);
                    (compositor.create_texture(f.w, f.h, &f.rgba), f.w, f.h)
                })
                .collect();
            app.set_cheat_faces(Some(stem), faces);
        }
        None => app.set_cheat_faces(None, Vec::new()),
    }
}

/// Reconciles the per-system screen overlay texture with `overlay_pixels`. Called every frame,
/// but does nothing unless `overlay_dirty` is set: on insert the GL resource is minted once,
/// and on a cart with no overlay (GBA, or a missing/corrupt PNG) any previous texture is
/// released. The decoded RGBA is owned by `App`; only the compositor touches the GL name.
fn sync_overlay(app: &mut App, compositor: &mut Compositor) {
    if !app.overlay_dirty() {
        return;
    }
    if let Some(old) = app.take_overlay_tex() {
        compositor.release_texture(old);
    }
    let tex = match app.overlay_pixels() {
        Some((rgba, w, h))
            if *w > 0
                && *h > 0
                && (rgba.len() as u64) >= (*w as u64) * (*h as u64) * 4 =>
        {
            Some(compositor.create_texture(*w, *h, rgba))
        }
        _ => None,
    };
    app.set_overlay_tex(tex);
    app.set_overlay_clean();
}

fn sync_core_picker(
    app: &mut App,
    compositor: &mut Compositor,
    builder: &FaceBuilder,
    asked: &mut Option<String>,
    board: &mut Option<TexId>,
    lid: &mut Option<TexId>,
    built: &mut Option<String>,
) {
    let highlighted = app.selected_stem().map(str::to_string);
    if highlighted.is_some() && *asked != highlighted {
        if let Some(cart) = app
            .carts()
            .iter()
            .find(|c| highlighted.as_deref() == Some(c.stem.as_str()))
        {
            builder.request(cart.clone());
        }
        *asked = highlighted.clone();
    }
    let Some(faces) = builder.take() else {
        return;
    };
    if highlighted.as_deref() != Some(faces.stem.as_str()) || *built == highlighted {
        return;
    }
    let board_id = upload_rgba(
        compositor,
        board,
        faces.board.w,
        faces.board.h,
        &faces.board.rgba,
    );
    let lid_id = upload_rgba(compositor, lid, faces.lid.w, faces.lid.h, &faces.lid.rgba);
    app.set_core_board_faces(board_id, lid_id);
    *built = Some(faces.stem);
}

fn upload(compositor: &mut Compositor, slot: &mut Option<TexId>, face: slot_ui::UndoFace) -> TexId {
    upload_rgba(compositor, slot, face.w, face.h, &face.rgba)
}

fn upload_rgba(
    compositor: &mut Compositor,
    slot: &mut Option<TexId>,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> TexId {
    match *slot {
        Some(id) => {
            compositor.update_texture(id, w, h, rgba);
            id
        }
        None => {
            let id = compositor.create_texture(w, h, rgba);
            *slot = Some(id);
            id
        }
    }
}
