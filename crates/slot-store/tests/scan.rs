mod common;

use common::tmp_root;
use slot_store::{scan, scan_cached, Platform};
use tempfile::TempDir;

fn write_rom(d: &TempDir, rel: &str, title: &str) {
    let mut rom = vec![0u8; 0x100];
    rom[0xa0..0xa0 + title.len()].copy_from_slice(title.as_bytes());
    std::fs::write(d.path().join("Games").join(rel), rom).expect("write rom");
}

fn write_png(d: &TempDir, rel: &str) {
    let path = d.path().join("Labels").join(rel);
    std::fs::create_dir_all(path.parent().expect("label has a parent")).expect("create dir");
    std::fs::write(path, b"\x89PNG\r\n\x1a\n").expect("write png");
}

fn with_headers(root: &std::path::Path) -> Vec<slot_store::Cart> {
    let mut carts = scan(root).unwrap();
    for cart in &mut carts {
        cart.read_header();
    }
    carts
}

#[test]
fn a_png_in_labels_is_paired_to_its_rom_by_stem() {
    let d = tmp_root();
    write_rom(&d, "GBA/Pokemon Emerald.gba", "POKEMON EMER");
    write_png(&d, "GBA/Pokemon Emerald.png");
    write_rom(&d, "GBA/Advance Wars.gba", "ADVANCEWARS");
    let carts = scan(d.path()).unwrap();
    assert_eq!(carts.len(), 2);
    assert_eq!(carts[0].stem, "Advance Wars");
    assert!(carts[0].label.is_none());
    assert!(
        carts[1].label.is_some(),
        "a label in Labels/ was not picked up"
    );
    let mut emerald = carts[1].clone();
    emerald.read_header();
    assert_eq!(emerald.title, "POKEMON EMER");
}

#[test]
fn a_label_pairs_with_its_rom_whatever_the_case_of_its_name() {
    let d = tmp_root();
    write_rom(&d, "GBA/Pokemon Emerald.gba", "POKEMON EMER");
    write_png(&d, "GBA/pokemon emerald.PNG");
    let carts = scan(d.path()).unwrap();
    assert_eq!(
        carts[0].label.as_deref(),
        Some(d.path().join("Labels/GBA/pokemon emerald.PNG").as_path())
    );
}

#[test]
fn a_scan_reads_names_and_leaves_the_rom_headers_for_later() {
    let d = tmp_root();
    write_rom(&d, "GBA/Advance Wars.gba", "ADVANCEWARS");
    let mut cart = scan(d.path()).unwrap().remove(0);
    assert!(cart.title.is_empty());
    cart.read_header();
    assert_eq!(cart.title, "ADVANCEWARS");
}

#[test]
fn scan_ignores_non_gba_files() {
    let d = tmp_root();
    write_rom(&d, "GBA/Real.gba", "REAL");
    std::fs::write(d.path().join("Games/GBA/notes.txt"), "hi").unwrap();
    assert_eq!(scan(d.path()).unwrap().len(), 1);
}

#[test]
fn an_appledouble_sidecar_is_not_shelved_as_a_cart() {
    let d = tmp_root();
    write_rom(&d, "GBA/Metroid Fusion.gba", "METROID");
    write_rom(&d, "GBA/._Metroid Fusion.gba", "METROID");
    let carts = scan(d.path()).unwrap();
    assert_eq!(carts.len(), 1, "an AppleDouble sidecar reached the shelf");
    assert_eq!(carts[0].stem, "Metroid Fusion");
}

#[test]
fn header_title_of_a_truncated_rom_is_none_not_a_panic() {
    let d = tmp_root();
    std::fs::write(d.path().join("Games/GBA/Tiny.gba"), [0u8; 8]).unwrap();
    let mut cart = scan(d.path()).unwrap().remove(0);
    cart.read_header();
    assert!(cart.title.is_empty());
}

#[test]
fn a_header_title_that_is_not_text_is_dropped_rather_than_mangled() {
    let d = tmp_root();
    let mut rom = vec![0u8; 0x100];
    rom[0xa0..0xac].copy_from_slice(&[0xffu8; 12]);
    std::fs::write(d.path().join("Games/GBA/Garbage.gba"), rom).unwrap();
    let mut cart = scan(d.path()).unwrap().remove(0);
    cart.read_header();
    assert!(cart.title.is_empty());
}

#[test]
fn a_root_with_no_games_directory_scans_as_empty() {
    let d = tempfile::tempdir().unwrap();
    assert!(scan(d.path()).unwrap().is_empty());
}

// The cache is what a boot calls when it wants headers, so everything the plain scan promises
// has to hold through it as well, and the two have to agree once the plain scan has read them.

#[test]
fn the_cache_agrees_with_a_full_scan() {
    let d = tmp_root();
    write_rom(&d, "GBA/Pokemon Emerald.gba", "POKEMON EMER");
    write_rom(&d, "GBA/Advance Wars.gba", "ADVANCEWARS");
    write_png(&d, "GBA/Pokemon Emerald.png");
    assert_eq!(scan_cached(d.path()).unwrap(), with_headers(d.path()));
}

/// The whole point: a card that has not changed must not go back to the roms. Rewriting the
/// title the cache holds leaves the header on disk saying something else, so a run that
/// returns the cache's answer is one that did not open a single rom.
#[test]
fn an_unchanged_card_is_answered_from_the_cache_without_reading_a_rom() {
    let d = tmp_root();
    write_rom(&d, "GBA/Emerald.gba", "POKEMON EMER");
    assert_eq!(scan_cached(d.path()).unwrap()[0].title, "POKEMON EMER");

    let cache = d.path().join("System/library.cache");
    let text = std::fs::read_to_string(&cache).expect("no cache written");
    std::fs::write(&cache, text.replace("POKEMON EMER", "FROM CACHE")).unwrap();

    assert_eq!(scan_cached(d.path()).unwrap()[0].title, "FROM CACHE");
}

#[test]
fn a_rom_added_after_the_first_scan_is_shelved() {
    let d = tmp_root();
    write_rom(&d, "GBA/Emerald.gba", "POKEMON EMER");
    assert_eq!(scan_cached(d.path()).unwrap().len(), 1);
    // Twelve bytes is the whole of the header title field, so this is a title rather than
    // the start of one.
    write_rom(&d, "GBA/Metroid.gba", "METROID");
    let carts = scan_cached(d.path()).unwrap();
    assert_eq!(carts.len(), 2);
    assert!(carts
        .iter()
        .any(|c| c.stem == "Metroid" && c.title == "METROID"));
}

#[test]
fn a_rom_removed_after_the_first_scan_leaves_the_shelf() {
    let d = tmp_root();
    write_rom(&d, "GBA/Emerald.gba", "POKEMON EMER");
    write_rom(&d, "GBA/Metroid.gba", "METROID");
    assert_eq!(scan_cached(d.path()).unwrap().len(), 2);
    std::fs::remove_file(d.path().join("Games/GBA/Metroid.gba")).unwrap();
    let carts = scan_cached(d.path()).unwrap();
    assert_eq!(carts.len(), 1);
    assert_eq!(carts[0].stem, "Emerald");
}

#[test]
fn a_label_added_after_the_first_scan_is_picked_up() {
    let d = tmp_root();
    write_rom(&d, "GBA/Emerald.gba", "POKEMON EMER");
    assert!(scan_cached(d.path()).unwrap()[0].label.is_none());
    write_png(&d, "GBA/Emerald.png");
    assert!(scan_cached(d.path()).unwrap()[0].label.is_some());
}

/// A cache that does not parse is a slow boot, not a failure to boot.
#[test]
fn a_corrupt_cache_is_a_full_scan_rather_than_an_error() {
    let d = tmp_root();
    write_rom(&d, "GBA/Emerald.gba", "POKEMON EMER");
    std::fs::write(
        d.path().join("System/library.cache"),
        b"\x00 not a cache at all",
    )
    .unwrap();
    let carts = scan_cached(d.path()).unwrap();
    assert_eq!(carts.len(), 1);
    assert_eq!(carts[0].title, "POKEMON EMER");
}

/// A cache missing a row would take a cart off the shelf even though nothing on the card
/// moved, so the row count is part of what makes it trustworthy: a `rows 2` header with one
/// row under it has to read as no cache rather than as a one cart shelf.
#[test]
fn a_cache_with_a_row_torn_out_is_a_full_scan() {
    let d = tmp_root();
    write_rom(&d, "GBA/Emerald.gba", "POKEMON EMER");
    write_rom(&d, "GBA/Metroid.gba", "METROID FUSION");
    assert_eq!(scan_cached(d.path()).unwrap().len(), 2);

    let cache = d.path().join("System/library.cache");
    let text = std::fs::read_to_string(&cache).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.pop();
    std::fs::write(&cache, lines.join("\n")).unwrap();

    assert_eq!(scan_cached(d.path()).unwrap().len(), 2);
}

/// A card whose games are named with a title that itself carries the cache's own separators
/// must round trip: a rom title is not ours to assume printable.
#[test]
fn a_title_with_a_separator_in_it_survives_the_cache() {
    let d = tmp_root();
    write_rom(&d, "GBA/Odd.gba", "ODD\tTITLE");
    assert_eq!(scan_cached(d.path()).unwrap()[0].title, "ODD\tTITLE");
    assert_eq!(scan_cached(d.path()).unwrap()[0].title, "ODD\tTITLE");
}

/// The letter ring files a cart under the name the shelf shows, which is the file name — never
/// the header title. The header is twelve characters of the publisher's language, so a card of
/// Chinese file names filed under it puts `POKEMON RUBY` and `ROCKMANZERO` on the ring and a
/// press of the dial lands on a letter with nothing under it.
#[test]
fn the_ring_files_a_cart_under_its_file_name_and_not_its_header() {
    let d = tmp_root();
    // A rom's header title says one thing and the file name another, which is the whole case.
    write_rom(&d, "GBA/宝可梦-红宝石 (CN).gba", "POKEMON RUBY");
    // Bracketed tags are not part of the name, and the letter has to see through them: this
    // cart is an `L`, not a `#`.
    write_rom(&d, "GBA/[GBA] 洛克人Zero1.gba", "ROCKMANZERO");
    write_rom(&d, "GBA/恶魔城-晓月圆舞曲.gba", "AKUMAJO");

    let mut carts = scan(d.path()).unwrap();
    // Grouped by letter, which is the shelf's order. The fast scan has not opened a rom.
    let filed: Vec<(&str, char)> = carts.iter().map(|c| (c.stem.as_str(), c.initial)).collect();
    assert_eq!(
        filed,
        vec![
            ("宝可梦-红宝石 (CN)", 'B'),
            ("恶魔城-晓月圆舞曲", 'E'),
            ("[GBA] 洛克人Zero1", 'L'),
        ]
    );
    assert!(carts.iter().all(|c| c.title.is_empty()));
    // And the header is still carried, for the things that want it.
    for cart in &mut carts {
        cart.read_header();
    }
    assert!(carts.iter().any(|c| c.title == "ROCKMANZERO"));
}

/// The same cart through the cache: `initial` is derived rather than stored, so it has to be
/// derived the same way from a row as from a rom. A cache that answered with a different letter
/// than the scan that wrote it would put the ring a boot out of step with the shelf.
#[test]
fn the_cache_files_a_cart_under_the_same_letter_as_the_scan() {
    let d = tmp_root();
    write_rom(&d, "GBA/宝可梦-红宝石 (CN).gba", "POKEMON RUBY");
    write_rom(&d, "GBA/[GBA] 洛克人Zero1.gba", "ROCKMANZERO");

    let first = scan_cached(d.path()).unwrap();
    let again = scan_cached(d.path()).unwrap();
    let letters = |cs: &[slot_store::Cart]| -> Vec<char> { cs.iter().map(|c| c.initial).collect() };
    assert_eq!(
        letters(&first),
        vec!['B', 'L'],
        "the first scan is already wrong"
    );
    assert_eq!(
        letters(&again),
        letters(&first),
        "the cache changed the letters"
    );
}

/// The shelf is grouped by letter, because the ring above it is an index and an index whose
/// entries are out of order is not one. Hanzi in code point order are not in pinyin order, so
/// the file name alone leaves the B carts scattered down the row and stepping the dial to C
/// lands behind the user.
#[test]
fn the_shelf_is_grouped_by_letter() {
    let d = tmp_root();
    // Written in an order that is neither: 宝 (B) sorts before 恶 (E) before 洛 (L) by code
    // point, and the file names below are handed over in the third of those orders.
    write_rom(&d, "GBA/恶魔城-晓月圆舞曲.gba", "AKUMAJO");
    write_rom(&d, "GBA/宝可梦-红宝石.gba", "POKEMON");
    write_rom(&d, "GBA/洛克人Zero1.gba", "ROCKMAN");
    write_rom(&d, "GBA/超级机器人大战A.gba", "SRW A");
    write_rom(&d, "GBA/宝可梦-蓝宝石.gba", "POKEMON");
    write_rom(&d, "GBA/(J) 洛克人Zero2.gba", "ROCKMAN");

    let carts = scan(d.path()).unwrap();
    let filed: Vec<(char, &str)> = carts.iter().map(|c| (c.initial, c.stem.as_str())).collect();
    assert_eq!(
        filed,
        vec![
            ('B', "宝可梦-红宝石"),
            ('B', "宝可梦-蓝宝石"),
            ('C', "超级机器人大战A"),
            ('E', "恶魔城-晓月圆舞曲"),
            ('L', "(J) 洛克人Zero2"),
            ('L', "洛克人Zero1"),
        ],
        "the shelf is not grouped by letter, or not by file name inside a letter"
    );
    // Non-decreasing, which is the property the dial's up and down depend on: down is always
    // forwards along the row and up is always back.
    assert!(carts.windows(2).all(|w| w[0].initial <= w[1].initial));
}

#[test]
fn a_cart_takes_the_platform_of_the_folder_it_is_in() {
    let d = tmp_root();
    write_rom(&d, "GBA/Metroid Fusion.gba", "METROID");
    std::fs::write(d.path().join("Games/GB/Tetris.gb"), vec![0u8; 0x150]).unwrap();
    std::fs::write(d.path().join("Games/GBC/Chromatic.gbc"), vec![0u8; 0x150]).unwrap();

    let carts = scan(d.path()).unwrap();

    let by_stem = |s: &str| carts.iter().find(|c| c.stem == s).unwrap().platform;
    assert_eq!(by_stem("Metroid Fusion"), Platform::Gba);
    assert_eq!(by_stem("Tetris"), Platform::Gb);
    assert_eq!(by_stem("Chromatic"), Platform::Gbc);
}

#[test]
fn a_gba_rom_in_the_game_boy_folder_does_not_appear() {
    let d = tmp_root();
    std::fs::write(d.path().join("Games/GB/Wrong.gba"), vec![0u8; 0x150]).unwrap();
    assert!(scan(d.path()).unwrap().is_empty());
}

#[test]
fn an_unreadable_platform_folder_does_not_take_the_rest_of_the_library_with_it() {
    let d = tmp_root();
    write_rom(&d, "GBA/Metroid Fusion.gba", "METROID");
    std::fs::write(d.path().join("Games/GB/Tetris.gb"), vec![0u8; 0x150]).unwrap();
    std::fs::remove_dir(d.path().join("Games/GBC")).unwrap();
    std::fs::write(d.path().join("Games/GBC"), b"not a directory").unwrap();

    let carts = scan(d.path()).expect("one bad folder must not fail the whole scan");

    assert_eq!(carts.len(), 2, "the readable shelves were lost too");
    assert!(carts.iter().any(|c| c.stem == "Metroid Fusion"));
    assert!(carts.iter().any(|c| c.stem == "Tetris"));
}

#[test]
fn the_same_stem_on_two_platforms_is_two_carts_with_two_labels() {
    let d = tmp_root();
    write_rom(&d, "GBA/Tetris.gba", "TETRIS");
    std::fs::write(d.path().join("Games/GB/Tetris.gb"), vec![0u8; 0x150]).unwrap();
    std::fs::create_dir_all(d.path().join("Labels/GB")).unwrap();
    std::fs::write(d.path().join("Labels/GB/Tetris.png"), b"\x89PNG\r\n\x1a\n").unwrap();

    let carts = scan(d.path()).unwrap();

    assert_eq!(carts.len(), 2);
    let gb = carts.iter().find(|c| c.platform == Platform::Gb).unwrap();
    let gba = carts.iter().find(|c| c.platform == Platform::Gba).unwrap();
    assert!(gb.label.is_some(), "the Game Boy label was not found");
    assert!(
        gba.label.is_none(),
        "the GBA cart borrowed the Game Boy label"
    );
}

/// A Game Boy cart keeps the Color/SGB facts the shelf asks for, and its letter still comes
/// from the file name. The fast scan does not open the rom to learn either of those except
/// when something asks.
#[test]
fn a_game_boy_cart_keeps_its_header_and_its_letter() {
    let d = tmp_root();
    let mut rom = vec![0u8; 0x150];
    rom[0x134..0x134 + 6].copy_from_slice(b"TETRIS");
    rom[0x146] = 0x03;
    std::fs::write(d.path().join("Games/GB/宝可梦.gb"), &rom).unwrap();

    let mut carts = scan(d.path()).unwrap();
    assert_eq!(carts.len(), 1);
    assert_eq!(carts[0].platform, Platform::Gb);
    assert_eq!(carts[0].system(), slot_store::System::Gb);
    assert_eq!(carts[0].initial, 'B');
    assert!(carts[0].title.is_empty(), "a fast scan opened the rom");
    assert!(carts[0].sgb_enhanced());
    carts[0].read_header();
    assert_eq!(carts[0].title, "TETRIS");
}
