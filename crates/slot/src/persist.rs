use std::path::{Path, PathBuf};
use std::time::SystemTime;

use slot_store::{
    atomic_write, describe, is_rzip, read_slot_state, save_plan, write_slot_state, Core, Platform,
    SavePlan, System,
};

pub trait Snapshot {
    fn state(&self) -> Option<Vec<u8>>;
    fn save_ram(&self) -> Option<Vec<u8>>;
    fn thumb(&self) -> Option<Vec<u8>>;
    fn load(&self, state: Vec<u8>);

    fn resume_trusted(&self) -> bool {
        true
    }

    fn save_ram_trusted(&self) -> bool {
        true
    }
}

pub fn flush(
    root: &Path,
    platform: Platform,
    core: Core,
    system: System,
    stem: &str,
    state: Option<&[u8]>,
    sav: Option<&[u8]>,
) -> std::io::Result<()> {
    if let Some(state) = state {
        write_resume(root, platform, core, system, stem, state)?;
    }
    if let Some(sav) = sav {
        write_sav(root, platform, system, stem, sav)?;
    }
    Ok(())
}

pub fn eject(
    root: &Path,
    platform: Platform,
    core: Core,
    system: System,
    stem: &str,
    state: Option<&[u8]>,
    sav: Option<&[u8]>,
) -> std::io::Result<()> {
    flush(root, platform, core, system, stem, state, sav)?;
    let mut slot = read_slot_state(root);
    slot.cart = None;
    slot.cart_platform = None;
    write_slot_state(root, &slot)
}

/// The core hands back the whole save ram whether or not the game touched it, so an
/// unchanged one is a rewrite of up to 128 KB of card for nothing.
///
/// What may be written is decided by `slot_store::sav::save_plan`. `BackupAndWrite` renames a
/// shorter filler file aside before the shorter save replaces it, and `RefuseShrink` /
/// `RefuseCompressed` leave a real or rzip save untouched. The bytes themselves are whatever
/// the core reported, RTC included.
pub fn write_sav(
    root: &Path,
    platform: Platform,
    system: System,
    stem: &str,
    sav: &[u8],
) -> std::io::Result<bool> {
    let path = sav_path(root, system, stem);
    let old = read_sav(root, platform, system, stem);
    let dropped = old
        .as_deref()
        .map_or(&[][..], |o| &o[sav.len().min(o.len())..]);
    match save_plan(old.as_deref(), sav) {
        SavePlan::Write => {}
        SavePlan::Unchanged => return Ok(false),
        SavePlan::Blank => return Ok(false),
        SavePlan::BackupAndWrite => {
            let from = found_sav_path(root, platform, system, stem);
            let to = backup_path(&from);
            match std::fs::rename(&from, &to) {
                Ok(()) => eprintln!(
                    "slot: save ram: {} was {} of filler past the {} bytes now reported; moved \
                     it to {} and wrote the shorter save",
                    from.display(),
                    describe(dropped),
                    sav.len(),
                    to.display()
                ),
                Err(e) => {
                    eprintln!(
                        "slot: save ram: could not move {} aside ({e}); refusing to shrink it \
                         from {} to {} bytes",
                        from.display(),
                        old.as_deref().map_or(0, <[u8]>::len),
                        sav.len()
                    );
                    return Ok(false);
                }
            }
        }
        SavePlan::RefuseShrink => {
            eprintln!(
                "slot: save ram: refusing to shrink {} from {} to {} bytes -- what would be \
                 dropped holds {}",
                found_sav_path(root, platform, system, stem).display(),
                old.as_deref().map_or(0, <[u8]>::len),
                sav.len(),
                describe(dropped)
            );
            return Ok(false);
        }
        SavePlan::RefuseCompressed => {
            eprintln!(
                "slot: save ram: refusing to overwrite the compressed RetroArch save {} \
                 (turn off RetroArch's save-file compression and re-export it)",
                found_sav_path(root, platform, system, stem).display()
            );
            return Ok(false);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    atomic_write(&path, sav)?;
    Ok(true)
}

/// mGBA standalone writes `.sav`, RetroArch's libretro cores write `.srm`. Both are the
/// same battery bytes. Machine folders (`Saves/GBA`, `Saves/GB`, `Saves/GBC`) are preferred;
/// a card that has not been sorted yet is still read from flat `Saves/`. Only `.sav` under the
/// machine folder is written.
pub fn read_sav(root: &Path, platform: Platform, system: System, stem: &str) -> Option<Vec<u8>> {
    for path in sav_candidates(root, platform, system, stem) {
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(bytes);
        }
    }
    None
}

/// What may be handed to a core, as opposed to `read_sav`, which is what is on the card.
/// A RetroArch rzip container is not save ram; loading nothing leaves the compressed file
/// alone for a later export.
pub fn load_sav(root: &Path, platform: Platform, system: System, stem: &str) -> Option<Vec<u8>> {
    let data = read_sav(root, platform, system, stem)?;
    if is_rzip(&data) {
        eprintln!(
            "slot: save ram: {} is a compressed RetroArch save; loading nothing rather than \
             feeding its rzip bytes to the core",
            found_sav_path(root, platform, system, stem).display()
        );
        return None;
    }
    Some(data)
}

fn backup_path(from: &Path) -> PathBuf {
    let name = from.file_name().and_then(|n| n.to_str()).unwrap_or("save");
    let size = std::fs::metadata(from).map_or(0, |m| m.len());
    let candidate = |n: usize| {
        let nth = if n == 1 {
            String::new()
        } else {
            format!("-{n}")
        };
        from.with_file_name(format!("{name}.bak-{size}{nth}"))
    };
    for n in 1..100 {
        let path = candidate(n);
        if !path.exists() {
            return path;
        }
    }
    candidate(1)
}

fn found_sav_path(root: &Path, platform: Platform, system: System, stem: &str) -> PathBuf {
    sav_candidates(root, platform, system, stem)
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| sav_path(root, system, stem))
}

fn sav_path(root: &Path, system: System, stem: &str) -> PathBuf {
    root.join("Saves")
        .join(system.dir_name())
        .join(format!("{stem}.sav"))
}

fn sav_candidates(root: &Path, platform: Platform, system: System, stem: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for name in [system.dir_name(), platform.dir_name()] {
        let dir = root.join("Saves").join(name);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.push(root.join("Saves"));
    let mut paths = Vec::new();
    for dir in dirs {
        paths.push(dir.join(format!("{stem}.sav")));
        paths.push(dir.join(format!("{stem}.srm")));
    }
    paths
}

/// Resume states live in two layouts. Upstream writes
/// `States/<platform>/<core>/<stem>/resume.state`; the fork writes
/// `States/<core>/<system>/<stem>/resume.state`. A card from before either sort keeps
/// `States/<core>/<stem>/resume.state`. The newest of the three is the one that is loaded.
/// A write updates both sorted layouts so neither reader is left behind.
pub fn read_resume(
    root: &Path,
    platform: Platform,
    core: Core,
    system: System,
    stem: &str,
) -> Option<Vec<u8>> {
    resume_dirs(root, platform, core, system, stem)
        .into_iter()
        .filter_map(|dir| read_resume_file(&dir))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, bytes)| bytes)
}

fn write_resume(
    root: &Path,
    platform: Platform,
    core: Core,
    system: System,
    stem: &str,
    state: &[u8],
) -> std::io::Result<()> {
    let dirs = resume_dirs(root, platform, core, system, stem);
    // The legacy flat directory is read, never written.
    for dir in dirs.into_iter().take(2) {
        std::fs::create_dir_all(&dir)?;
        atomic_write(&dir.join("resume.state"), state)?;
    }
    Ok(())
}

fn resume_dirs(
    root: &Path,
    platform: Platform,
    core: Core,
    system: System,
    stem: &str,
) -> [PathBuf; 3] {
    [
        root.join("States")
            .join(platform.dir_name())
            .join(core.as_str())
            .join(stem),
        root.join("States")
            .join(core.as_str())
            .join(system.dir_name())
            .join(stem),
        root.join("States").join(core.as_str()).join(stem),
    ]
}

fn read_resume_file(dir: &Path) -> Option<(SystemTime, Vec<u8>)> {
    let path = dir.join("resume.state");
    let bytes = std::fs::read(&path).ok()?;
    let modified = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Some((modified, bytes))
}
