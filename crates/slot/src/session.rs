use std::path::PathBuf;
use std::time::Duration;

use slot_input::{Action, Gestures, Millis, RawEvent};
use slot_retro::Rumble;
use slot_store::{Core, Platform, System};
use slot_ui::{FfState, Toast};

pub const RUMBLE_MIN_MS: Millis = 100;
pub const RUMBLE_MAX_MS: Millis = 150;

use crate::app::{App, Phase};
use crate::audio::{open_sink, AudioSink, Profile, Ring, Sfx, GBA_HZ};
use crate::core::open_core;
use crate::emu::{CoreState, EmuHandle, Speed};
use crate::frames::FrameRef;
use crate::input::Pad;
use crate::persist;
use crate::prefetch::Prefetch;

pub struct Session {
    root: PathBuf,
    app: App,
    emu: Option<EmuHandle>,
    sink: Box<dyn AudioSink>,
    /// The profile the sink was opened for. Applied when the next cart is inserted.
    audio: Profile,
    gestures: Gestures,
    pad: Pad,
    rewinding: bool,
    fast: bool,
    motor: u16,
    pulse: Option<(Millis, u16)>,
    prefetch: Prefetch,
    reloading: bool,
    driven: bool,
}

impl Session {
    pub fn boot(root: PathBuf) -> Self {
        Self::boot_with(root, open_sink())
    }

    pub fn boot_with(root: PathBuf, mut sink: Box<dyn AudioSink>) -> Self {
        let audio = crate::root::audio_profile(&root);
        // A frontend for one console knows the rate before it knows the cart. A device that
        // refuses it still opens, and the worker resamples to whatever it did take.
        if let Err(e) = sink.open(GBA_HZ, audio) {
            eprintln!("slot: audio: {e}");
        }
        let pad_remap = crate::root::remap(&root).unwrap_or_default();
        // Timed, because a device with no console turns every boot-time change into a matter
        // of opinion otherwise: the log is the only place the numbers exist.
        let _ = std::fs::write(root.join("System/boot.log"), "");
        let scan_at = std::time::Instant::now();
        let app = App::boot(&root);
        let scan_ms = scan_at.elapsed().as_millis();
        let session = Session {
            app,
            root,
            emu: None,
            sink,
            audio,
            gestures: Gestures::new(),
            pad: Pad::with_remap(pad_remap),
            rewinding: false,
            fast: false,
            motor: 0,
            pulse: None,
            prefetch: Prefetch::default(),
            reloading: false,
            driven: false,
        };
        session.boot_note(&format!("library scan + shelf {}", scan_ms));
        session
    }

    pub fn play_sfx(&mut self, sfx: Sfx) {
        let ring = self.sink.ring();
        let rate = ring.sample_rate();
        if rate == 0 {
            return;
        }
        let mut samples = sfx.render(rate);
        crate::audio::volume::apply(&mut samples, self.app.output_volume());
        ring.mix(&samples);
    }

    pub fn audio_queued(&self) -> usize {
        self.sink.ring().queued_frames()
    }

    pub fn audio_ring(&self) -> std::sync::Arc<Ring> {
        self.sink.ring()
    }

    pub fn rumble(&mut self, strength: u16) {
        if strength == self.motor {
            return;
        }
        self.motor = strength;
        self.app.set_rumble(strength);
    }

    pub fn prefetching(&self) -> Option<&std::path::Path> {
        self.prefetch.reading()
    }

    pub fn core_rumble(&self) -> Option<&Rumble> {
        self.emu.as_ref().map(EmuHandle::rumble)
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn emu(&self) -> Option<&EmuHandle> {
        self.emu.as_ref()
    }

    pub fn set_driven(&mut self, driven: bool) {
        self.driven = driven;
        if let Some(emu) = &self.emu {
            emu.set_driven(driven);
        }
    }

    pub fn step_emulator(&self, present: Duration, timeout: Duration) -> bool {
        self.emu.as_ref().is_some_and(|emu| {
            emu.tick(present);
            emu.wait_frame(timeout)
        })
    }

    pub fn frame(&self) -> Option<FrameRef> {
        self.emu.as_ref().and_then(|e| e.latest_frame())
    }

    pub fn core_settling(&self) -> bool {
        self.emu
            .as_ref()
            .is_some_and(|e| e.state() == CoreState::Loading)
    }

    pub fn has_core(&self) -> bool {
        self.emu.is_some()
    }

    pub fn frame_ready(&self) -> bool {
        self.emu.as_ref().is_some_and(EmuHandle::frame_ready)
    }

    pub fn frames_published(&self) -> u64 {
        self.emu.as_ref().map_or(0, EmuHandle::published_count)
    }

    pub fn observed_speed(&self) -> Option<Speed> {
        self.emu.as_ref().map(EmuHandle::observed_speed)
    }

    pub fn frames_taken(&self) -> u64 {
        self.emu.as_ref().map_or(0, EmuHandle::frames_taken)
    }

    pub fn game_visible(&self) -> bool {
        self.app.game_visible()
    }

    pub fn feed(&mut self, events: impl IntoIterator<Item = RawEvent>, now: Millis) {
        let mut actions = Vec::new();
        for ev in events {
            actions.extend(self.gestures.feed(ev, now));
        }
        actions.extend(self.gestures.tick(now));
        for action in actions {
            self.act(action);
        }
        self.sync_pad();
    }

    fn sync_pad(&mut self) {
        for btn in self.app.taken_buttons() {
            self.pad.apply(Action::GbaUp(*btn));
        }
        if let Some(emu) = &self.emu {
            emu.set_input(self.pad.mask());
        }
    }

    fn bridge_link(&mut self, f: impl FnOnce(&mut App)) {
        let had_link = self.app.link_active();
        f(&mut self.app);
        if had_link && !self.app.link_active() {
            if let Some(emu) = &self.emu {
                emu.end_link();
            }
        }
    }

    fn act(&mut self, action: Action) {
        if trace() {
            eprintln!("slot: {action:?} in {:?}", self.app.phase());
        }
        // A system-shelf swap blanks the buttons: nothing reaches `App` and the pad is not
        // updated, so the movement plays out untouched. The volume and mute side of `act` is
        // behind this too, which is the point — a level change mid-swap is a key that landed.
        if self.app.input_locked() {
            return;
        }
        match action {
            Action::RewindStart => self.rewinding = true,
            Action::RewindStop => self.rewinding = false,
            Action::FfStart => self.fast = true,
            Action::FfStop => self.fast = false,
            Action::CheatToggle => {
                // SELECT+A opens the cheat table (the browsable, per-code list) rather than only
                // flipping a master flag. Pressed again while the table is up it flips every code
                // at once — the old master toggle, now reachable from inside.
                if self.app.cheat_menu().is_some() {
                    self.app.toggle_all_cheats();
                    self.push_cheats();
                    self.app.toast_now(if self.app.cheats_on {
                        Toast::CheatsOn
                    } else {
                        Toast::CheatsOff
                    });
                } else {
                    self.app.open_cheat_menu();
                }
                return;
            }
            _ => {}
        }
        let menu = self.overlaid();
        self.bridge_link(|app| app.apply(action));
        // A row toggled inside the cheat table set a flag the table itself cannot act on, because
        // it has no reach to the emulator. Push the list now that `apply` has run.
        if self.app.take_cheats_dirty() {
            self.push_cheats();
        }
        // After apply: the level the sink wants is the one the action just produced.
        if matches!(
            action,
            Action::VolumeUp | Action::VolumeDown | Action::MuteToggle
        ) {
            if let Some(emu) = &self.emu {
                emu.set_volume(self.app.output_volume());
            }
        }
        if menu || self.overlaid() {
            self.pad.clear();
        } else if self.app.takes_from_the_game(action) {
            if let Action::GbaDown(btn) | Action::GbaUp(btn) = action {
                self.pad.apply(Action::GbaUp(btn));
            }
        } else {
            self.pad.apply(action);
        }
        self.sync_rumble();
    }

    pub fn update(&mut self, dt: f32) {
        self.bridge_link(|app| app.update(dt));
        if let Some(emu) = &self.emu {
            emu.set_volume(self.app.output_volume());
        }
        if let Some((client_id, transport)) = self.app.take_link_transport() {
            match &self.emu {
                Some(emu) => match self.app.link_player() {
                    Some(player) => emu.begin_cable(player, transport),
                    None => emu.begin_link(client_id, transport),
                },
                None => eprintln!("slot: link: a transport arrived with no core to run it"),
            }
        }
        if let Some(p) = self.app.take_gb_palette() {
            if let Some(emu) = &self.emu {
                emu.set_option("mgba_gb_colors", p.core_name());
            }
        }
        if let Some(on) = self.app.take_colour_correction() {
            if let Some((key, value)) = crate::core::colour_option(self.app.core(), on) {
                if let Some(emu) = &self.emu {
                    emu.set_option(key, value);
                }
            }
        }
        if let Some((stem, serial)) = self.app.take_link_reload() {
            self.reload_for_link(&stem, serial);
        }
        if self.app.link_active() {
            if self.emu.as_ref().is_some_and(EmuHandle::bios_mismatch) {
                self.bridge_link(|app| app.bios_mismatch());
            } else if self.emu.as_ref().is_some_and(EmuHandle::peer_ended) {
                self.bridge_link(|app| app.peer_ended());
            } else if self.emu.as_ref().is_some_and(EmuHandle::link_lost) {
                self.app.peer_lost();
            }
        }
        if let Some(sfx) = self.app.take_sfx() {
            self.play_sfx(sfx);
        }
        self.sync_core();
        self.sync_reload();
        if !self.has_core() {
            for action in self.gestures.drop_ff_latch() {
                self.act(action);
            }
        }
        let ready = self.emu.as_ref().is_some_and(EmuHandle::has_published);
        if ready && !self.app.game_ready() {
            launch("first frame shown");
        }
        self.app.set_game_ready(ready);
        self.sync_speed();
        self.sync_rewind_hud();
        self.sync_ff_hud();
        self.sync_rumble();
        self.sync_audio_profile();
        self.sync_pad();
        let now = self.app.now();
        self.prefetch.browse(self.app.browsed_rom(), now);
    }

    /// The shelf is the only screen that can change the audio profile, and the only one where
    /// applying it is free: the device buffer is fixed when the PCM is opened, so a change
    /// means reopening the hardware. Guarded on `playing()` as well, so that even a stray
    /// chord cannot reopen the device under a running game.
    fn sync_audio_profile(&mut self) {
        let wanted = self.app.audio_profile();
        if wanted == self.audio || self.playing() {
            return;
        }
        self.audio = wanted;
        if let Err(e) = self.sink.open(GBA_HZ, wanted) {
            eprintln!("slot: audio: {e}");
        }
        eprintln!("slot: audio profile {}", wanted.as_str());
    }

    /// Append a line to `System/boot.log`, which `boot` truncates at every start. The device
    /// has no console, so a boot being tuned has to leave its timings where a PC can read
    /// them — and a card is the one thing every one of these has.
    /// How many carts the shelf is holding. Read only to label the face timing, so the count
    /// and the duration can be read off the log together.
    pub fn cart_count(&self) -> usize {
        self.app.carts().len()
    }

    pub fn boot_note(&self, line: &str) {
        use std::io::Write;
        let path = self.root.join("System/boot.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{line}");
        }
    }

    fn sync_rumble(&mut self) {
        let asked = match &self.emu {
            Some(emu) if self.playing() && self.app.rumble_enabled() => emu.rumble().strength(),
            _ => {
                self.pulse = None;
                return self.rumble(0);
            }
        };
        let now = self.app.now();
        let want = match (asked, self.pulse) {
            (0, Some((at, strength))) if now.saturating_sub(at) < RUMBLE_MIN_MS => strength,
            (0, _) => {
                self.pulse = None;
                0
            }
            (asked, Some((at, _))) => {
                self.pulse = Some((at, asked));
                if now.saturating_sub(at) < RUMBLE_MAX_MS {
                    asked
                } else {
                    0
                }
            }
            (asked, None) => {
                self.pulse = Some((now, asked));
                asked
            }
        };
        self.rumble(want);
    }

    fn sync_ff_hud(&mut self) {
        let ff = match (self.actually_fast_forwarding(), self.gestures.ff_latched()) {
            (false, _) => FfState::Off,
            (true, false) => FfState::Held,
            (true, true) => FfState::Latched,
        };
        self.app.set_ff(ff);
    }

    fn sync_rewind_hud(&mut self) {
        let fill = self
            .actually_rewinding()
            .then(|| self.emu.as_ref().map(EmuHandle::rewind_fill))
            .flatten();
        match fill {
            Some(fill) => self.app.show_rewind(fill),
            None => self.app.hide_rewind(),
        }
    }

    fn actually_rewinding(&self) -> bool {
        self.rewinding && self.playing() && self.app.may_rewind()
    }

    fn actually_fast_forwarding(&self) -> bool {
        self.fast && self.playing() && self.app.may_fast_forward()
    }

    fn inserting(&self) -> bool {
        matches!(self.app.phase(), Phase::Inserting { .. })
    }

    fn showing_polaroids(&self) -> bool {
        matches!(self.app.phase(), Phase::Polaroids { .. })
    }

    fn overlaid(&self) -> bool {
        self.showing_polaroids() || self.held()
    }

    fn playing(&self) -> bool {
        matches!(self.app.phase(), Phase::Playing { .. }) && !self.held()
    }

    fn held(&self) -> bool {
        self.app.power_menu().is_some()
            || self.app.game_menu_open()
            || self.app.shutting_down()
            || self.app.cheat_menu().is_some()
            || self.app.palette_browser().is_some()
    }

    fn dozing(&self) -> bool {
        matches!(self.app.phase(), Phase::Doze { .. })
    }

    fn ejecting(&self) -> bool {
        matches!(self.app.phase(), Phase::Ejecting { .. })
    }

    fn sync_speed(&self) {
        if let Some(emu) = &self.emu {
            emu.set_fast_steps(u32::from(self.app.ff_speed()));
            emu.set_ff_sound(self.app.ff_sound());
            emu.set_speed(
                if self.inserting()
                    || self.ejecting()
                    || self.showing_polaroids()
                    || self.dozing()
                    || (self.held() && !self.app.link_active())
                {
                    Speed::Paused
                } else if self.actually_fast_forwarding() {
                    Speed::Fast
                } else {
                    Speed::Normal
                },
            );
            emu.set_rewinding(self.actually_rewinding());
            emu.set_turbo(self.app.turbo());
            emu.set_rewind_recording(self.app.records_rewind());
        }
    }

    fn no_core() -> bool {
        std::env::var_os("SLOT_NO_CORE").is_some_and(|v| v != "0")
    }

    fn sync_core(&mut self) {
        if Self::no_core() {
            return;
        }
        let stem = match self.app.phase() {
            Phase::Shelf => {
                self.emu = None;
                return;
            }
            Phase::Inserting { cart, .. } => cart.clone(),
            _ => return,
        };
        if self.emu.is_none() {
            let (_, serial) = self.app.link_mode(&stem);
            self.spawn_core(&stem, serial);
        }
        match self.emu.as_ref().map(EmuHandle::state) {
            Some(CoreState::Loading) => {}
            Some(CoreState::Ready) => {
                if let (true, Some(p), Some(emu)) =
                    (self.app.palette_live(), self.app.gb_palette(), &self.emu)
                {
                    emu.set_option("mgba_gb_colors", p.core_name());
                }
                self.app.on_core_ready()
            }
            Some(CoreState::Failed) | None => {
                self.emu = None;
                self.app.on_core_failed();
            }
        }
    }

    fn spawn_core(&mut self, stem: &str, serial: &'static str) {
        let Some((rom, platform)) = self
            .app
            .seated_cart()
            .filter(|c| c.stem == stem)
            .map(|c| (c.rom.clone(), c.platform))
        else {
            return;
        };
        let system = match platform {
            Platform::Gba => System::Gba,
            Platform::Gb => System::Gb,
            Platform::Gbc => System::Gbc,
        };
        // gpSP does the Game Boy Advance and nothing else, so anything that is not one has
        // exactly one core that can take it, whatever the card's own file says.
        let core = match platform {
            Platform::Gba => slot_store::core_for_platform(&self.root, stem, platform),
            Platform::Gb | Platform::Gbc => Core::Mgba,
        };
        launch(&format!("opening {stem} on {}", core.as_str()));
        self.app.set_core(core);
        self.app.set_platform(platform);
        self.app
            .set_video_mode(crate::video_mode::video_mode_for(&self.root, stem));
        self.app.set_link_loaded(serial);
        if matches!(platform, Platform::Gb | Platform::Gbc) && slot_store::sgb_enhanced(&rom) {
            eprintln!("slot: {stem} is SGB-enhanced; mGBA Super Game Boy border disabled");
        }
        let resume = (!self.app.starting_clean())
            .then(|| persist::read_resume(&self.root, platform, core, system, stem))
            .flatten();
        launch("resume state read");
        let player = self.app.link_player();
        let palette = crate::core::palette_for(self.app.gb_palette(), platform, &rom, player);
        self.app.set_palette_live(palette.is_some());
        let opened = open_core(
            &self.root,
            core,
            system,
            serial,
            self.app.colour_correction(),
            palette,
            player,
        );
        self.app.set_named_core(opened.named);
        launch("core opened");
        let sav = persist::load_sav(&self.root, platform, system, stem);
        launch("save read");
        let ring = self.sink.ring();
        let emu = match player.filter(|_| platform == Platform::Gba) {
            Some(p) => EmuHandle::spawn_linked(opened.core, rom, ring, sav, resume, p, self.audio),
            None => EmuHandle::spawn(opened.core, rom, ring, sav, resume, self.audio),
        };
        emu.set_volume(self.app.output_volume());
        emu.set_driven(self.driven);
        self.app.set_snapshot(Box::new(emu.snapshot()));
        self.emu = Some(emu);
        // Load this cart's cheat list off the card into `App`, then push it to the core. Codes
        // default to OFF (see `App::set_cheats`), so a cart that ships codes launches with
        // nothing applied; the player opens the table (SELECT+A) and switches on the ones they
        // want. SELECT+A while the table is open flips the master `cheats_on` and re-pushes.
        self.app.set_cheats(crate::root::cheats(&self.root, stem));
        self.push_cheats();
        // The screen-overlay set for the cart's system (`Overlay/gb*.png` / `gbc*.png`), with
        // the persisted rotation index applied: slot 0 is GB, 1 is GBC. GBA has no overlay set
        // at all, so it clears whatever the previous cart left on screen — a Game Boy's model
        // art (or a GBC's frame) must not bleed through into a GBA game, and there is no key on
        // this machine that would turn it off. The frontend mints the GL texture when it next
        // sees `overlay_dirty`.
        let (which, slot) = match system {
            System::Gb => ("gb", 0),
            System::Gbc => ("gbc", 1),
            System::Gba => ("", 0),
        };
        if which.is_empty() {
            // No overlay set for this system: drop the previous cart's overlay.
            eprintln!("slot: overlay: GBA has no set — clearing");
            self.app.clear_overlay_set();
        } else {
            let list = crate::root::overlay_files(&self.root, which);
            // Say what the card actually offered. Art the scanner did not take — a name it does
            // not recognise — is otherwise invisible: the device has no console, and the overlay
            // leaves no other trace anywhere on screen or off.
            let names: Vec<String> = list
                .iter()
                .map(|o| {
                    format!(
                        "{}{}",
                        o.path.file_name().unwrap_or_default().to_string_lossy(),
                        if o.reflect { " (reflect)" } else { "" }
                    )
                })
                .collect();
            eprintln!(
                "slot: overlay {which}: {} file(s): {}",
                list.len(),
                if names.is_empty() {
                    "(none - check Overlay/ naming)".to_string()
                } else {
                    names.join(", ")
                }
            );
            self.app.set_overlay_set(list, slot);
        }
    }

    /// (Re)send the current cheat list to the core, each code's enabled ANDed with the master
    /// `cheats_on`. Cheap: a handful of `retro_cheat_set` calls, and a no-op for a core without
    /// cheats (gpSP exports no `retro_cheat_set`, so each call no-ops).
    fn push_cheats(&mut self) {
        if let Some(emu) = &self.emu {
            let list: Vec<(bool, String)> = self
                .app
                .cheats()
                .iter()
                .map(|c| (self.app.cheats_on && c.enabled, c.code.clone()))
                .collect();
            emu.set_cheats(list);
        }
    }

    fn reload_for_link(&mut self, stem: &str, serial: &'static str) {
        eprintln!("slot: link: loading {stem} again with gpsp_serial={serial}");
        if self.emu.is_some() {
            self.app.flush_resume();
        }
        self.emu = None;
        self.spawn_core(stem, serial);
        self.reloading = true;
    }

    fn sync_reload(&mut self) {
        if !self.reloading {
            return;
        }
        match self.emu.as_ref().map(EmuHandle::state) {
            Some(CoreState::Loading) => {}
            Some(CoreState::Ready) => {
                self.reloading = false;
                self.app.link_reload_done();
            }
            Some(CoreState::Failed) | None => {
                self.reloading = false;
                self.emu = None;
                self.app.link_reload_failed();
                if let Some((stem, serial)) = self.app.take_link_reload() {
                    self.reload_for_link(&stem, serial);
                }
            }
        }
    }
}

pub(crate) fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SLOT_TRACE").is_some())
}

fn launch(step: &str) {
    eprintln!("slot: launch: {step} at {:.1} ms", crate::boot_time::ms());
}
