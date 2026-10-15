# AniCraft

An open-source 2D animation editor in pure Rust. AniCraft opens and saves Adobe Animate `XFL` and `FLA` files, edits
vector, bitmap, text and symbol timelines with classic, motion and shape tweens, and exports `SWF`, animated `GIF`,
`MP4`, `PNG` sequences and `SVG`. It runs on macOS, Windows and Linux and has a Model Context Protocol (MCP)
interface so agents can drive the same commands a person uses.

The engine lives in [anicraft-libs](https://github.com/soof-golan/anicraft-libs); the sibling project for live
character animation is [CharCraft](https://github.com/soof-golan/charcraft). Both are built on the open-source
[Craft](https://github.com/storytold) crates.

See [AGENTS.md](AGENTS.md) for the rules every change follows.

## Crates

| Crate | What |
|---|---|
| `anicraft-swf` | SWF writer: header, tags, shapes with gradients and bitmap fills, sprites, sounds, from display lists |
| `anicraft-xfl` | XFL folder and zipped FLA reader/writer on the `anim-scene` model |
| `anicraft-tools` | Drawing and selection tools as pure functions from pointer events to commands |
| `anicraft-mcp` | MCP server over the application command registry |
| `anicraft-app` | The desktop app (`anicraft`) |
| `anicraft-cli` | Headless convert, render and export |

## Building

```sh
cargo test --workspace
```

Rust 1.99.1 is pinned in `rust-toolchain.toml`.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [BSD-3-Clause](LICENSE-BSD-3-Clause), at your option. Third-party code
and assets are listed in [ATTRIBUTION.md](ATTRIBUTION.md).

This project is not affiliated with or endorsed by Adobe. Adobe, Animate, Flash and SWF are trademarks or
registered trademarks of Adobe Inc.; they are named here only to describe file compatibility.
