mod art;
pub use art::cover;
mod backdrop;
mod barcode;
mod battery;
mod board;
mod cart;
mod cart_art;
mod clock;
mod draw;
mod hud;
mod icon;
pub mod lang;
pub mod letters;
mod link_art;
pub mod palette;
mod palette_browser;
mod plate;
mod polaroids;
mod power_menu;
mod quick_menu;
mod rebuild;
mod refusal;
mod shelf;
mod shell;
mod shortcuts;
mod silhouette;
mod slot_chrome;
mod status;
mod sticker;
pub mod text;
mod toast;

pub use backdrop::{bezel_face, draw_backdrop, wallpaper_face};
pub use barcode::{code39, CODE39_NARROW, CODE39_WIDE};
pub use battery::{cluster_h, draw_gauge, BOLT_H, BOLT_PX, BOLT_W};
pub use board::{
    board_at, board_face, board_from, board_zoom, chip_face, chip_shadow_face, grown, lid_at,
    lid_from, lift_of, on_board, padded, rom_marking, rom_marking_face, shelf_cart, shelf_cart_at,
    slide_of, socket_face, Placed, BOARD_H, BOARD_W, BOARD_X, BOARD_Y, CHIP_H, CHIP_TIP, CHIP_U,
    CHIP_V, CHIP_W, HOP_LIFT, LID_TURN, ROM_H, ROM_W, ROM_X, ROM_Y, SHADOW_H, SHADOW_W,
    SLIDE_SHARE, SLIDE_UP, SOCKET_H, SOCKET_U, SOCKET_V, SOCKET_W, TURN_PAD,
};
pub use cart::face_profile;
pub use cart_art::{
    board_art, install as install_cart_art, label_share as cart_label_share,
    load as load_cart_art, summary as cart_art_summary,
};
// Re-exported rather than defined here: the rule that turns a file name into a title belongs
// with the scan, because the ring's letter is decided from it and a scan cannot ask the ui.
pub use slot_store::{clean_label, label_tags};

pub use cart::{
    cart_box, cart_face, cart_face_with, cart_placeholder, cart_shadow, gb_cart_shadow,
    gb_label_panel, gb_shell_of, label_colour, label_panel, label_size, label_text, seated_box,
    size_for, CartFace, CartSize, CART_H, CART_W, FACE_H, FACE_SCALE, FACE_W, GB_CART_H, GB_CART_W,
    SEATED_W,
    GB_LABEL_H, GB_LABEL_W, GB_LABEL_X, GB_LABEL_Y, LABEL_H, LABEL_W, LABEL_X, LABEL_Y,
};
pub use clock::{clock_label, date_time_text, hhmm, set_clock_hint_face, ClockPicker, Field};
pub use draw::{Draw, TexId, OUT_H, OUT_W};
// The entry point for reading a PNG the user dropped on the card (e.g. a screen overlay).
// Exposed so `slot` can decode `Overlay/<sys>/<sys>.png` without the rest of `art` going public.
pub use art::decode_rgba;
pub use hud::{
    badge_at, ff_badge, FfState, Hud, HudKind, LinkBadge, Millis, ALARM_BAND_H, HUD_ICON_PX,
    HUD_INK, HUD_MS, LINK_HOST_INK, LINK_JOIN_INK, PLATE_H, PLATE_Y,
};
pub use icon::{badge_face, icon_box, icon_face, warm_fonts, Badge, Icon};
pub use letters::Letters;
pub use link_art::{
    link_art, LinkArt, ADAPTER_BASE_X, ADAPTER_BASE_Y, ADAPTER_H, ADAPTER_W, ARCS, ARROW_H,
    ARROW_LEFT_X, ARROW_RIGHT_X, ARROW_W, ARROW_Y, CLICKS_H, CLICKS_W, CLICKS_X, CLICKS_Y, PLUG_H,
    PLUG_TIP_X, PLUG_W, PORT_H, PORT_W, PORT_Y,
};
pub use palette::Mode;
pub use palette_browser::{
    cell_width as palette_cell_w, draw as draw_palette_browser, name_face as palette_name_face,
    name_height as palette_name_h, pages_of as palette_pages, rows_of as palette_rows,
    Swatch as PaletteSwatch, View as PaletteView, COLS as PALETTE_COLS, FOOT_PX as PALETTE_FOOT_PX,
    PER_PAGE as PALETTE_PER_PAGE, ROWS as PALETTE_ROWS, STOPS as PALETTE_STOPS,
};
pub use plate::{
    arrows_hint_face, arrows_hint_width, cap_width, centred_hints, clock_face, clock_width,
    dialog_line_face, hint_face, hint_quad, hint_row, hint_width, shelf_title_face, title_face,
    word_face, word_width, Hint, UndoFace, ARROW_GAP, CAP, CAP_GAP, HINT_EDGE, HINT_GAP, HINT_H,
    LEGEND_GAP, SHELF_TITLE_H, SHELF_TITLE_W, STATUS_H, TITLE_H, TITLE_W,
};
pub use polaroids::{photo_face, PhotoFace, Polaroids, DOT, LEGEND, PHOTO_H, PHOTO_W};
pub use power_menu::{cheat_row_face, menu_face, PowerChoice, MENU_PAD};
pub use quick_menu::{
    quick_caret_face, quick_label_face, quick_legend_faces, quick_value_face, QuickMenu,
    QuickMenuFaces, QuickRow, QuickValue, QUICK_EDGE, QUICK_PITCH, QUICK_SPLIT, QUICK_TOP,
};
pub use rebuild::{rebuild_count_face, rebuild_title_face, RebuildScreen};
pub use refusal::Refusal;
pub use shelf::{
    foot_y, rest_y, set_shelf_slack, Motion, Shelf, SlotRect, CENTER_SCALE, SIDE_ALPHA,
};
pub use shell::{
    gb_table_shells, gba_shell_for, lookup_order_is_exact_then_family_then_default, shell_for,
    shell_presets, table_keys, Finish, Shell, DEFAULT_SHELL, DMG_SHELL, DUAL_MODE_SHELL,
    GB_CLEAR_SHELL,
};
pub use shortcuts::{
    draw_row as draw_shortcut_row, hint_face as shortcut_hint_face, row_face as shortcut_row_face,
    Row as ShortcutRow, HINT as SHORTCUT_HINT, ROWS as SHORTCUT_ROWS, ROW_W as SHORTCUT_ROW_W,
};
pub use silhouette::{gb_silhouette, silhouette, GbShell};
pub use slot_chrome::{
    draw_edge_glow, draw_empty_slot, draw_slot_name, draw_top_band, ease, edge, housing, opening,
    recess, set_theme, SlotChrome, ALERT_PX, LIP_H, MOUTH_H, MOUTH_W, TOP_BAND_H, TOP_WIN_H,
    TOP_WIN_W, TOP_WIN_X, TOP_WIN_Y,
};
pub use status::{draw_status, Printed};
pub use sticker::{
    draw_sticker, draw_sticker_at, head_rows, sticker_face, sticker_lines, StickerFields,
    COPYRIGHT, CREDITS, DC, HOME, ORIGIN, STICKER_H, STICKER_W,
};
pub use toast::{toast_box, toast_face, toast_rect, Toast};
