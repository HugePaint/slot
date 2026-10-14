use std::collections::HashMap;
use std::fmt;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::atomic::atomic_write;
use crate::gba::{header_title_code, sgb_enhanced};
use crate::platform::Platform;

/// Which machine a cart is for, and so which shelf it belongs on.
///
/// Three shelves, because that is how a card is filed and how a hand asks for one. Decided off
/// the folder the scan found the rom in — `Games/GBA`, `Games/GB`, `Games/GBC` — which is the
/// same answer `Platform` gives. The extension helpers stay for callers that only have a name.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum System {
    Gba,
    Gb,
    Gbc,
}

impl System {
    /// From whatever follows the last dot in a file name. Compared without case, because a
    /// card written on another machine arrives in whatever case that machine felt like.
    pub fn from_extension(ext: &str) -> Option<Self> {
        if ext.eq_ignore_ascii_case("gba") {
            Some(System::Gba)
        } else if ext.eq_ignore_ascii_case("gb") {
            Some(System::Gb)
        } else if ext.eq_ignore_ascii_case("gbc") {
            Some(System::Gbc)
        } else {
            None
        }
    }

    /// The folder this machine's carts and faces go in, on a card that sorts them.
    pub fn dir_name(self) -> &'static str {
        match self {
            System::Gba => "GBA",
            System::Gb => "GB",
            System::Gbc => "GBC",
        }
    }

    pub fn from_platform(platform: Platform) -> Self {
        match platform {
            Platform::Gba => System::Gba,
            Platform::Gb => System::Gb,
            Platform::Gbc => System::Gbc,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cart {
    pub platform: Platform,
    pub stem: String,
    pub rom: PathBuf,
    pub label: Option<PathBuf>,
    pub title: String,
    pub code: String,
    /// Which bucket the shelf's letter ring files this cart under: the pinyin initial of the
    /// name the shelf shows, tags stripped. Derived at scan time from the file name, so a fast
    /// scan that has not opened the rom still has a letter. See `bucket`.
    pub initial: char,
    pub shell: Option<crate::cart_shell::ShellChoice>,
}

impl Cart {
    /// Which machine this cart is for. The folder it was scanned from, not a second guess off
    /// the extension: a `.gb` filed under `Games/GBC` belongs to the Color shelf.
    pub fn system(&self) -> System {
        System::from_platform(self.platform)
    }

    /// Whether this cart's rom is Super Game Boy enhanced — mGBA would draw an SGB border
    /// for it unless SLOT turns that off. Reads the rom on demand, so it is not part of the
    /// scan fast path; calling it per cart at boot would cost a header read apiece.
    pub fn sgb_enhanced(&self) -> bool {
        sgb_enhanced(&self.rom)
    }

    pub fn read_header(&mut self) {
        match self.platform {
            Platform::Gba => {
                if let Some((title, code)) = header_title_code(&self.rom) {
                    self.title = title;
                    self.code = code;
                }
            }
            Platform::Gb | Platform::Gbc => {
                if let Some(header) = crate::gb::header(&self.rom) {
                    self.title = header.title;
                    self.code = header.code;
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

/// The shelf, from the names in `Games/<platform>/`.
///
/// Headers stay unread. A boot that only needs the file name, the letter and the label pays
/// for a directory listing, not a cluster walk per rom; `read_header` fills the title and the
/// code when something actually wants them, and `scan_cached` is the path that remembers those
/// reads on the card.
///
/// One unreadable platform folder is logged and skipped. The other shelves still come up.
pub fn scan(root: &Path) -> Result<Vec<Cart>, StoreError> {
    let shells = load_shells(root);
    let mut carts = Vec::new();
    for platform in Platform::ALL {
        let dir = root.join("Games").join(platform.dir_name());
        let entries = match std::fs::read_dir(&dir) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("slot: scan: {}: {e}", dir.display());
                continue;
            }
        };
        let labels_dir = root.join("Labels").join(platform.dir_name());
        let labels = listing(&labels_dir);
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let rom = entry.path();
            if is_hidden(&rom) || !platform.accepts(&rom) || !is_file(&entry) {
                continue;
            }
            let Some(stem) = rom.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string()) else {
                continue;
            };
            let label = labels
                .get(&format!("{stem}.png").to_lowercase())
                .map(|name| labels_dir.join(name));
            carts.push(Cart {
                platform,
                stem: stem.clone(),
                rom,
                label,
                title: String::new(),
                code: String::new(),
                initial: bucket(&stem),
                shell: shells.get(&crate::cart_shell::key(&stem)).copied(),
            });
        }
    }
    order(&mut carts);
    Ok(carts)
}

/// The shelf with headers filled, re-read only where the card has changed.
///
/// `scan` deliberately does not open roms. This is the path that does, once, and writes the
/// titles to `System/library.cache` so the next boot of an unchanged card does not open them
/// again. A name whose size and timestamp are the ones the cache already has keeps the header
/// it came with, so a card with one new game pays for one game.
///
/// What it does not catch is a rom rewritten in place under its own name with its own
/// timestamp untouched. `System/library.cache` is a plain file: delete it to force the slow
/// path.
pub fn scan_cached(root: &Path) -> Result<Vec<Cart>, StoreError> {
    let mut carts = scan(root)?;
    let games = game_keys(&carts);
    let labels = label_names(root);
    let stamp = Stamp {
        games: dir_mtime(&root.join("Games")),
        labels: dir_mtime(&root.join("Labels")),
    };
    let cached = read_cache(root);
    if let Some(c) = &cached {
        if c.stamp == stamp && c.games == games && c.labels == labels && c.rows_agree() {
            apply_rows(&mut carts, &c.rows);
            return Ok(carts);
        }
    }

    let old = cached.as_ref().map(|c| c.rows.as_slice());
    let mut rows = Vec::with_capacity(carts.len());
    for cart in &mut carts {
        let name = game_key(cart);
        let meta = std::fs::metadata(&cart.rom).ok();
        let (size, mtime) = meta.as_ref().map_or((0, 0), |m| (m.len(), mtime_secs(m)));
        let known = old.and_then(|rows| {
            rows.iter()
                .find(|r| r.name == name && r.size == size && r.mtime == mtime)
        });
        if let Some(row) = known {
            cart.title = row.title.clone();
            cart.code = row.code.clone();
        } else {
            cart.read_header();
        }
        rows.push(Row {
            name,
            size,
            mtime,
            has_label: cart.label.is_some(),
            title: cart.title.clone(),
            code: cart.code.clone(),
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let games = rows.iter().map(|r| r.name.clone()).collect();
    write_cache(
        root,
        &Cache {
            stamp,
            games,
            labels,
            rows,
        },
    );
    Ok(carts)
}

fn load_shells(root: &Path) -> HashMap<String, crate::cart_shell::ShellChoice> {
    let read = |file: &str| std::fs::read_to_string(root.join(file)).unwrap_or_default();
    crate::cart_shell::layered(
        &read(crate::cart_shell::CART_SHELL_FILE),
        &read(crate::cart_shell::LABELS_SHELL_FILE),
    )
}

pub fn listing(dir: &Path) -> HashMap<String, String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashMap::new();
    };
    entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .map(|name| (name.to_lowercase(), name))
        .collect()
}

fn is_file(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(t) if t.is_symlink() => entry.path().is_file(),
        Ok(t) => t.is_file(),
        Err(_) => false,
    }
}

/// Digits, then letters, then everything else. The letter ring itself files by `initial`;
/// this is the coarser key a caller sorts a bare list of titles with.
pub fn sort_key(stem: &str) -> (u8, String) {
    (group_of(stem), stem.to_uppercase())
}

fn group_of(stem: &str) -> u8 {
    match stem.chars().find(|c| !c.is_whitespace()) {
        Some(c) if c.is_ascii_digit() => 0,
        Some(c) if c.is_alphabetic() => 1,
        _ => 2,
    }
}

/// The bucket a title belongs in, for a caller that already has the string the shelf shows.
///
/// Leading space is skipped, a Latin letter is its own bucket, a hanzi is its pinyin initial,
/// and digits, punctuation and kana are `#`. Bracketed tags are not stripped here — that is
/// `bucket`, which is what the scan stores on the cart.
pub fn initial(stem: &str) -> char {
    crate::pinyin::initial(stem.trim_start())
}

/// Which letter ring bucket a cart belongs in.
///
/// From the name the shelf *shows*, which is the file name with its tags taken off — not from
/// the header title. The header title is twelve characters of the publisher's language, so a
/// card of Chinese file names filed under it puts POKEMON EMER and ROCKMANZERO on the ring and
/// jumps to a letter that matches nothing anybody can see.
fn bucket(stem: &str) -> char {
    crate::pinyin::initial(&crate::name::clean_label_stripped(stem))
}

/// By machine, then by letter, then by file name.
///
/// The machine comes first because the three shelves are three lists. Within one, the letter
/// comes first because the ring above the shelf is an index, and an index is only an index if
/// its entries are in order. File names are hanzi, and hanzi in code point order have nothing
/// to do with the order they are read in, so a shelf sorted by file name has its B carts
/// scattered down the row. Grouped, the row reads A, B, C left to right.
fn order(carts: &mut [Cart]) {
    carts.sort_by(|a, b| {
        (a.platform as u8)
            .cmp(&(b.platform as u8))
            .then(a.initial.cmp(&b.initial))
            .then_with(|| a.stem.cmp(&b.stem))
            .then_with(|| a.rom.cmp(&b.rom))
    });
}

// ---------------------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------------------

/// Under `System/` with the rest of slot's own bookkeeping. It describes the card rather than
/// the shelf the user curates, so it does not live beside the roms.
const CACHE: &str = "System/library.cache";

/// The first line, which is also the format's version. A stale stamp or a run of bytes that
/// only looks like one has to read as no cache at all rather than as a shelf with holes in
/// it, and the version is what makes that possible across a future change to the layout.
const MAGIC: &str = "slot-library 1";

/// The directory timestamps the names below were gathered at. Kept beside the names rather
/// than inferred: an empty directory and a directory that could not be read are both "no
/// names", and only the timestamp says which of the two the cache was written for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Stamp {
    games: i64,
    labels: i64,
}

struct Row {
    /// `GBA/Emerald.gba` — the path under `Games/`. The stem is derived from it rather than
    /// stored, so one name cannot disagree with the label it looks up.
    name: String,
    size: u64,
    mtime: i64,
    has_label: bool,
    title: String,
    code: String,
}

struct Cache {
    stamp: Stamp,
    /// Sorted. Compared as a whole: the order is part of the identity, and a cache written in
    /// another order is one this code did not write.
    games: Vec<String>,
    labels: Vec<String>,
    rows: Vec<Row>,
}

impl Cache {
    /// Whether the rows account for every game the cache claims. A truncated or hand-edited
    /// file can pass the name comparison and still be missing a row, which would take a cart
    /// off the shelf until the next change happened to rewrite it.
    fn rows_agree(&self) -> bool {
        self.rows.len() == self.games.len()
            && self
                .rows
                .iter()
                .zip(&self.games)
                .all(|(row, name)| row.name == *name)
    }
}

fn cache_path(root: &Path) -> PathBuf {
    root.join(CACHE)
}

fn read_cache(root: &Path) -> Option<Cache> {
    let text = std::fs::read_to_string(cache_path(root)).ok()?;
    parse_cache(&text)
}

/// Best effort. A card that will not take the write still boots, it only boots slowly — which
/// is what it did before the cache existed, so there is nothing here worth failing over.
fn write_cache(root: &Path, cache: &Cache) {
    let _ = atomic_write(&cache_path(root), render_cache(cache).as_bytes());
}

fn render_cache(cache: &Cache) -> String {
    let mut out = String::new();
    out.push_str(MAGIC);
    out.push('\n');
    section(&mut out, "games", cache.stamp.games, &cache.games);
    section(&mut out, "labels", cache.stamp.labels, &cache.labels);
    out.push_str(&format!("rows {}\n", cache.rows.len()));
    for row in &cache.rows {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            escape(&row.name),
            row.size,
            row.mtime,
            u8::from(row.has_label),
            escape(&row.title),
            escape(&row.code),
        ));
    }
    out
}

/// `<key> <stamp> <count>` followed by `count` escaped names, the shape `take_section` reads.
fn section(out: &mut String, key: &str, stamp: i64, names: &[String]) {
    out.push_str(&format!("{key} {stamp} {}\n", names.len()));
    for name in names {
        out.push_str(&escape(name));
        out.push('\n');
    }
}

fn parse_cache(text: &str) -> Option<Cache> {
    let mut lines = text.lines();
    if lines.next()? != MAGIC {
        return None;
    }
    let (games_stamp, games) = take_section(&mut lines, "games")?;
    let (labels_stamp, labels) = take_section(&mut lines, "labels")?;
    let count = lines.next()?.strip_prefix("rows ")?.parse().ok()?;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let mut fields = lines.next()?.split('\t');
        let row = Row {
            name: unescape(fields.next()?),
            size: fields.next()?.parse().ok()?,
            mtime: fields.next()?.parse().ok()?,
            has_label: fields.next()? == "1",
            title: unescape(fields.next()?),
            code: unescape(fields.next()?),
        };
        if fields.next().is_some() {
            return None;
        }
        rows.push(row);
    }
    Some(Cache {
        stamp: Stamp {
            games: games_stamp,
            labels: labels_stamp,
        },
        games,
        labels,
        rows,
    })
}

/// `<key> <stamp> <count>` followed by `count` escaped names.
fn take_section<'a>(lines: &mut std::str::Lines<'a>, key: &str) -> Option<(i64, Vec<String>)> {
    let rest = lines.next()?.strip_prefix(key)?.strip_prefix(' ')?;
    let (stamp, count) = rest.split_once(' ')?;
    let count: usize = count.parse().ok()?;
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        names.push(unescape(lines.next()?));
    }
    Some((stamp.parse().ok()?, names))
}

/// The three bytes that would break the line format written out of the way. A card is the
/// user's and a rom's header text is not ours to assume printable, so neither a file name nor
/// a title is trusted to be free of them. Escaping rather than rejecting keeps a cart on the
/// shelf instead of quietly dropping it from the cache.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // An escape this code did not write. Kept verbatim rather than dropped, so a
            // hand-edited file loses nothing it can be read back for.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn game_key(cart: &Cart) -> String {
    let name = cart
        .rom
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    format!("{}/{}", cart.platform.dir_name(), name)
}

fn game_keys(carts: &[Cart]) -> Vec<String> {
    let mut names: Vec<String> = carts.iter().map(game_key).collect();
    names.sort();
    names
}

fn label_names(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for platform in Platform::ALL {
        let dir = root.join("Labels").join(platform.dir_name());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_png(&path) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            names.push(format!("{}/{}", platform.dir_name(), name));
        }
    }
    names.sort();
    names
}

fn apply_rows(carts: &mut [Cart], rows: &[Row]) {
    for cart in carts {
        let name = game_key(cart);
        if let Some(row) = rows.iter().find(|r| r.name == name) {
            cart.title = row.title.clone();
            cart.code = row.code.clone();
            // The column is part of the row format. The label itself is taken from the
            // directory listing, which is newer than a cache written before the png arrived.
            let _ = row.has_label;
        }
    }
}

fn dir_mtime(dir: &Path) -> i64 {
    std::fs::metadata(dir).map_or(0, |m| mtime_secs(&m))
}

/// Seconds, or 0 for a filesystem that will not say. Zero is a value the stamp can hold and
/// compare like any other: a card whose directory timestamps are unreadable simply never
/// matches, which is a slow boot rather than a wrong shelf.
fn mtime_secs(m: &Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

fn is_png(p: &Path) -> bool {
    !is_hidden(p)
        && p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

pub fn is_hidden(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}
