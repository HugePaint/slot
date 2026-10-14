mod bezel;
mod draw;
mod fbdev;
mod fbo;
mod gl;
mod grade;
#[cfg(target_os = "macos")]
mod headless;
#[cfg(feature = "host")]
mod host;
mod lcd3x;
mod pipeline;
mod power;
mod quad;
mod shaders;
mod surface;
mod system;

pub use bezel::{framed, lifted_rect, screen_rect};
pub use draw::{Draw, TexId};
pub use fbdev::{egl_error, panel_mode, panel_size, FbdevSurface};
pub use fbo::{canvas_size, Compositor, BACKDROP};
pub use grade::{blue_light_gain, BLUE_LIGHT_MAX};
#[cfg(target_os = "macos")]
pub use headless::HeadlessSurface;
#[cfg(feature = "host")]
pub use host::HostSurface;
pub use lcd3x::{
    builtin_panel_mask, lcd3x_factors, lcd3x_mask, mask_texture_rgba8, ScreenEffect,
};
pub use pipeline::{SRC_H, SRC_W, WHOLE_TEXTURE};
pub use power::{screen_brightness, screen_rect_in, screen_scale, screen_width};
pub use surface::{blit_rect, blit_rect_fit, fit_rect, fit_scale, GfxError, Surface, OUT_H, OUT_W};
pub use system::{System, SCALE};
