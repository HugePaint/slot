mod atomic;
pub mod cart_shell;
mod config;
mod core;
pub mod gb;
mod gb_palette;
mod gba;
pub mod ini;
mod migrate;
mod name;
mod paging;
mod pinyin;
mod platform;
mod ring;
mod sav;
mod scan;
mod slot_state;
mod stamp;
mod theme;

pub use atomic::atomic_write;
pub use cart_shell::{Outline, ShellChoice, ShellFinish, CART_SHELL_FILE, LABELS_SHELL_FILE};
pub use config::{move_config, CONFIG_DIR};
pub use core::{
    core_for, core_for_platform, read_selected_cores, write_selected_core, Core, SELECTED_CORE_FILE,
};
pub use gb_palette::GbPalette;
pub use gba::{header, header_clean, header_code, header_title, header_title_code, sgb_enhanced};
pub use migrate::{migrate_states, MigrationReport};
pub use name::{clean_label, clean_label_stripped, init_label_config, label_tags, set_strip_tags};
pub use paging::{cycle_on_page, index_of, len_on, pages, step_cursor, step_page};
pub use platform::Platform;
pub use ring::{StateEntry, StateRing, RING_MAX};
pub use sav::{describe, is_rzip, save_plan, SavePlan, BLANK};
pub use scan::{
    initial, is_hidden, listing, scan, scan_cached, sort_key, Cart, StoreError, System,
};
pub use slot_state::{
    read_slot_state, write_slot_state, Mode, Shader, SlotState, BLUE_LIGHT_MAX, BRIGHTNESS_MAX,
    FF_SPEEDS, FF_SPEED_DEFAULT, UTC_OFFSET_MAX, UTC_OFFSET_MIN, VOLUME_MAX,
};
pub use stamp::{
    civil_from_days, days_from_civil, days_in_month, format_stamp, parse_stamp, stamp_now,
};
pub use theme::{Theme, THEME_FILE};
