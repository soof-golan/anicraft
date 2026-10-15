# anicraft-cli

Headless AniCraft: read Adobe Animate documents (`.fla`, XFL folders), render and export them, convert between
containers, and serve them to AI agents over the Model Context Protocol. Everything the GUI app will do to a document
goes through the same library code, so what this tool renders is what the editor shows.

Frame numbers are **1-based** everywhere a person or an agent sees them (like Animate's timeline). Scene and layer
indices are 0-based, layer 0 is the top layer.

## Commands

```text
anicraft-cli info FILE [--json] [--layers]
anicraft-cli render FILE --frame N [--scene NAME | --symbol NAME] [--scale X] [--transparent] -o out.png
anicraft-cli export FILE --format png-seq|gif|mp4|mov|svg|svg-anim|spritesheet|swf -o OUT
                   [--scene NAME] [--from N] [--to N] [--fps F] [--scale X] [--transparent|--alpha] [-q]
anicraft-cli convert IN OUT            # OUT.fla is a zip; any other name is an XFL folder (scene.xfl is a folder too)
anicraft-cli mcp [FILE] [--root DIR]...
```

* `export --format png-seq` writes `frame_0001.png`... into the folder `OUT`; `spritesheet` writes `OUT` (a PNG) and a
  JSON file next to it; `svg` is a still of the first frame of the range.
* `mp4` (H.264 + AAC) and `mov` (ProRes 4444, alpha with `--alpha`) include the document's sound layers, mixed with
  `anim-audio`. Sounds start at their keyframe, repeat as the loop setting says and stop at a Stop cue.
* `--fps` resamples by time, so a 12 fps document exports at 24 fps with every frame doubled.

## MCP

`anicraft-cli mcp` speaks MCP over stdin/stdout (log output goes to stderr). It is the shared `anim-mcp` front-end over
a headless session; see `docs/mcp.md` in anicraft-libs for the architecture (an actor owns the document, the MCP handler
only holds a channel to it).

```json
{ "mcpServers": { "anicraft": { "command": "anicraft-cli", "args": ["mcp", "/path/to/file.fla", "--root", "/path/to/work"] } } }
```

Without `--root` the client cannot open, save or export anything (renders still come back as images). With roots, all
file access is confined to them, symlinks resolved.

| Kind | Names |
|---|---|
| Tools from `anim-mcp` | `session_status`, `inspect_document`, `get_library`, `get_property`, `set_property`, `render_frame` (PNG image content; `options.symbol` previews a library symbol), `export` (progress and cancellation), `run_batch` (atomic, one undo step), `undo`, `redo`, `open_document`, `new_document`, `save_document`, `journal`, `list_commands`, `run_command`, `list_queries`, `query` |
| Command tools (one undo step each) | `document_set_stage`, `scene_add/remove/rename`, `layer_add/remove/rename/move`, `timeline_insert_keyframe`, `timeline_insert_blank_keyframe`, `timeline_clear_keyframe`, `timeline_insert_frames`, `timeline_remove_frames`, `timeline_create_classic_tween`, `timeline_create_shape_tween`, `timeline_remove_tween`, `timeline_set_ease`, `timeline_set_label`, `stage_add_symbol_instance`, `stage_remove_elements`, `library_rename_item`, `library_remove_item`, `library_duplicate_symbol` |
| Query tools | `query_stats`, `query_validate`, `query_keyframes`, `query_references`, `query_sound_cues` |
| Resources | `anim://session/status`, `anim://document/outline`, `anim://document/library`, `anim://commands`, `anim://session/journal`, `anim://property/{path}` |

Properties (`get_property` / `set_property`): `/stage/width`, `/stage/height`, `/stage/frame_rate`,
`/scenes/0/name`, `/scenes/0/layers/2/visible|locked|outline|name`.

Where the code lives: `src/mcp/commands.rs` (editing commands, plain functions over the document with JSON Schemas),
`src/mcp/mod.rs` (the `AppHooks` implementation: outline, library, properties, queries, render, export, open, save),
`src/mcp/doc.rs` (the session document: model, media, sidecars). A running editor embeds `anim_mcp::ControlServer`
with the same registry and hooks instead of the headless actor.
