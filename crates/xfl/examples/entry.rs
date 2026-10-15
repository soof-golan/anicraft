//! Prints (part of) one archive entry as text.
//!
//! `cargo run -p anicraft-xfl --example entry -- FILE ENTRY [SKIP_BYTES] [TAKE_BYTES]`
//!
//! With no `ENTRY` ("-"), lists the entries and their sizes.

use anim_archive::Container;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: entry FILE ENTRY [SKIP] [TAKE]")?;
    let name = args.next().unwrap_or_else(|| "-".to_owned());
    let skip: usize = args.next().map_or(Ok(0), |s| s.parse())?;
    let take: usize = args.next().map_or(Ok(4000), |s| s.parse())?;
    let container = Container::open(&path)?;
    if name == "-" {
        for entry in container.files() {
            println!("{:>12}  {}", entry.size, entry.name);
        }
        return Ok(());
    }
    let bytes = container.read(&name)?;
    let end = skip.saturating_add(take).min(bytes.len());
    let part = bytes.get(skip.min(end)..end).unwrap_or_default();
    println!("{}", String::from_utf8_lossy(part));
    Ok(())
}
