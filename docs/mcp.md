# AniCraft over MCP

Everything a person can do in AniCraft an agent can do through the Model Context Protocol, with the same commands, the
same undo history and the same renderer. There are two ways to connect:

| Mode | Command | What the agent drives |
|---|---|---|
| Headless | `anicraft-cli mcp [--file F] [--root DIR] [--read-root DIR] [--write-root DIR]` | A document in a background actor. No window. |
| Bridge | `anicraft --control-port 7421` and `anicraft-cli mcp --bridge 7421 [--token-file F]` | The running editor window. The agent's edits appear on screen and in the person's undo history, and `screenshot` captures the real window. |

`.mcp.json` at the repository root has an entry for each (`anicraft`, `anicraft-window`).

## Architecture

```text
 MCP client --stdio--> anicraft-cli mcp ---+--- headless: actor thread owns SessionHost<AniDoc, AniHooks>
                                           '--- bridge:   TCP 127.0.0.1 + token --> editor's ControlServer
                                                          --> ControlInbox pumped by the UI thread each frame
```

* `anim-mcp` (in anicraft-libs) is the generic server: JSON-RPC front end, tools generated from the `CommandRegistry`,
  journaling, background jobs with progress and cancellation, path confinement.
* This repository supplies the application part. `apps/anicraft-cli/src/mcp/` has the commands (`commands.rs`: stage,
  scenes, layers, frames, tweens, library; `editing.rs`: Modify/Insert, instance properties, text, sounds, import), the
  hooks (`mod.rs`: outline, library, properties, queries, render, export, open, save) and the document type. The tool
  commands (`draw_*`, `select_*`, `transform_*`, `paint_*`) live in `crates/tools` and drive the same tool state machines
  the mouse does. The window adds `selection.set_property` and screenshots (`apps/anicraft/src/mcp.rs`).
* Nothing is shared or locked: the document has one owner (the actor, or the UI thread) and everything else sends
  messages.

## Conventions

* **Frames are 1-based** everywhere an agent sees them (like the timeline). Scene, layer and element indices are 0-based;
  layer 0 is the top layer.
* Every command is one undo step. A failing command changes nothing. `run_batch` applies several atomically as one step.
* Colours are `#rrggbb` or `#rrggbbaa`. Lengths are stage pixels.
* Errors are tool errors with `{ "error": { "kind", "message" } }`; kinds: `invalid_params`, `failed`, `not_found`,
  `denied`, `unsupported`, `cancelled`.
* Right-to-left text (Hebrew, Arabic) and mixed text can be typed with `draw_text` in typing order; bidi runs are ordered
  by the Unicode algorithm when drawn and exported. `text_set_style` can force `direction` and `align`.

## Tool catalogue

Names are the command ids with `.` replaced by `_`. `list_commands` returns them with menu path, shortcut and enabled
state; every tool carries a JSON Schema and a description in `tools/list`.

| Area | Tools |
|---|---|
| Session | `session_status`, `journal`, `undo`, `redo`, `run_command`, `run_batch`, `list_commands` |
| Files | `new_document`, `open_document`, `save_document` (`.fla`), `library_import` (PNG, JPEG, GIF, BMP, WebP, PSD, AI, PDF, SVG) |
| Queries | `inspect_document` (outline), `get_library`, `get_property`, `set_property`, `query_timeline` (layer and frame table of a scene or a symbol), `query_element` (all properties of one object by path), `query_keyframes`, `query_references`, `query_sound_cues`, `query_stats`, `query_validate`, `stage_elements`, `tools_state`, `list_queries`, `query` |
| Pictures | `render_frame` (PNG image content, any frame, scale, or `options.symbol`), `screenshot` (window or one panel; bridge only) |
| Export | `export` with `format` = `png-seq`, `gif`, `mp4`, `mov`, `svg`, `svg-anim`, `spritesheet`, `swf`; progress and cancellation; `list_export_formats` |
| Document | `document_set_stage`, `scene_add`, `scene_remove`, `scene_rename` |
| Layers | `layer_add`, `layer_remove`, `layer_rename`, `layer_move`, `layer_duplicate`, `layer_set_state` (visible, locked, outline, folder/mask parent) |
| Timeline | `timeline_insert_keyframe`, `timeline_insert_blank_keyframe`, `timeline_clear_keyframe`, `timeline_insert_frames`, `timeline_remove_frames`, `timeline_move_keyframe`, `timeline_create_classic_tween`, `timeline_create_shape_tween`, `timeline_remove_tween`, `timeline_set_ease`, `timeline_set_tween_options` (rotate, turns, scale, snap, orient to path, sync), `timeline_set_label`, `timeline_set_sound`, `timeline_set_script`, `editor_goto` |
| Drawing tools | `draw_rectangle`, `draw_oval`, `draw_line`, `draw_path` (SVG path data or points), `draw_freehand` (pencil, brush), `draw_text`, `paint_bucket`, `paint_ink`, `paint_erase`, `paint_eyedrop`, and the raw `tools_use` event script |
| Tool options, colour | `tools_options` (merge patch: brush mode, shape and smoothing, eraser shape and faucet, bucket gap and lock fill, per-corner radii, oval arc, snapping), `colour_apply` (solid, gradient or bitmap fill and stroke style, applied to the selected shapes), `colour_swap`, `colour_defaults`, `swatches_list`, `swatches_add`, `swatches_delete`, `swatches_set` (also how `.aco` and `.act` files are loaded), `swatches_use`; the Gradient Transform tool is `tools_select` `gradient_transform` plus `tools_use` |
| Selection, transform | `select_at`, `select_rect`, `select_all`, `select_none`, `transform_move`, `transform_scale`, `transform_rotate`, `transform_skew`, `transform_matrix`, `edit_delete`, `edit_duplicate`, `edit_copy`, `edit_enter`, `edit_exit` |
| Objects | `modify_group`, `modify_ungroup`, `modify_break_apart`, `modify_arrange`, `modify_flip`, `modify_convert_to_symbol`, `insert_symbol`, `stage_add_symbol_instance`, `stage_remove_elements` |
| Instances | `instance_set_color_effect` (none, brightness, tint, alpha, advanced), `instance_add_filter` (blur, drop shadow, glow, colour matrix), `instance_clear_filters`, `instance_set_options` (blend mode, name, visible, cache as bitmap, graphic loop mode and first frame) |
| Text | `text_set_style` (text, face, size, colour, bold, italic, letter spacing, direction, alignment) |
| Library | `library_rename_item`, `library_remove_item`, `library_duplicate_symbol` |
| Window only | `selection_set_property` (any Properties panel field), the `properties` query (what it accepts right now) |

### Properties

`selection_set_property` takes `{"property": id, "value": ...}` and is one undo step, like dragging the field in the
Properties panel. The `properties` query (`query` with `name: "properties"`) lists every id the call accepts for the
current selection, with its label, section, current value and, for choices, the option names. Choices take the option's
name or its index, colours `#rrggbb` or `#rrggbbaa`, percentages are 0 to 100.

| Selection | Ids |
|---|---|
| Any object | `x`, `y`, `w`, `h`, `rotation` |
| Shape, drawing object | `fill`, `fill_alpha`, `fill_stopN` (gradients), `stroke`, `stroke_alpha`, `stroke_width`, `stroke_style` (solid, dashed, dotted), `stroke_cap`, `stroke_join`, `stroke_miter`, `stroke_scale`, `stroke_hinting`, `drawing_object` |
| Symbol instance | `symbol` (swap), `name`, `kind`, `visible`, `cache_as_bitmap`, `loop`, `first_frame`, `last_frame`, `effect`, `brightness`, `tint`, `tint_amount`, `alpha`, `adv_r` ... `adv_a_off`, `blend`, `filter.add`, `filter.N.*` (blur, drop shadow and glow parameters, `remove`) |
| Text | `text_type`, `name`, `orientation`, `auto_expand`, `text`, `font`, `size`, `text_color`, `text_alpha`, `bold`, `italic`, `letter_spacing`, `align`, `direction`, `line_spacing`, `indent`, `margin_left`, `margin_right` |
| Bitmap | `source` (swap), `bitmap.smoothing`, `bitmap.compression`, `bitmap.quality` |
| Group | `x`, `y`, `w`, `h`, `rotation` |
| Document | `doc.width`, `doc.height`, `doc.fps`, `doc.background` |
| Layer under the playhead | `layer.name`, `layer.visible`, `layer.locked`, `layer.outline`, `layer.color` |
| Keyframe under the playhead | `frame.label`, `frame.label_type`, `tween.type` (none, classic, shape), `tween.preset`, `tween.ease`, `tween.rotate`, `tween.rotate_times`, `tween.scale`, `tween.snap`, `tween.orient`, `tween.sync` |

Resources: `anim://session/status`, `anim://document/outline`, `anim://document/library`, `anim://commands`,
`anim://session/journal`, `anim://property/{path}`.

Object commands act on the selection, or on one object when `layer`, `frame` and `index` are all given.

## Example session

```text
new_document
document_set_stage        {"width":480,"height":270,"frame_rate":12,"background":"#fff6e5"}
draw_oval                 {"x":300,"y":40,"width":120,"height":120,"fill":"#2f7de1","object_drawing":true}
draw_text                 {"x":200,"y":190,"text":"שלום world مرحبا","size":26}
timeline_insert_frames    {"layer":0,"frame":1,"count":23}          # hold the backdrop for 24 frames
layer_add                 {"name":"card"}
editor_goto               {"layer":0,"frame":1}
draw_rectangle            {"x":30,"y":30,"width":140,"height":90,"fill":"#e85d3f","object_drawing":true}
select_at                 {"x":100,"y":70}
modify_convert_to_symbol  {"name":"Card","kind":"movie_clip"}
instance_add_filter       {"kind":"drop_shadow","distance":6,"blur_x":8,"blur_y":8}
timeline_insert_keyframe  {"layer":0,"frame":24}
editor_goto               {"frame":24}
select_at                 {"x":100,"y":70}
transform_move            {"dx":120,"dy":40}
timeline_create_classic_tween {"layer":0,"frame":1}
render_frame              {"frame":12}                              # a PNG comes back as image content
export                    {"format":"gif","path":"/work/card.gif"}
save_document             {"path":"/work/card.fla"}
```

A classic tween needs exactly one object on its layer, so put each moving object on its own layer.

## Security model

* The server speaks only on stdio (headless) or on `127.0.0.1` with a random token (the editor's control port). The token
  file is created with mode 0600 and the bridge reads it; there is no remote access. The default token file for
  `--control-port PORT` is `anicraft-control-PORT.token` in `$XDG_RUNTIME_DIR` or the temp folder.
* **Files are confined.** A headless server can read only inside `--root`/`--read-root` folders and write only inside
  `--root`/`--write-root` folders; without any, `open_document`, `save_document`, `export`, `library_import` and
  `render_frame` with a `path` are denied (renders still come back as images). Paths are resolved (symlinks, `..`)
  before the check. The editor applies the same rule to its `--root` folders and the folder of the opened file.
* Every call is journaled with its origin (`mcp`), outcome and time (`journal`).
* Documents are hostile input: readers cap sizes and depth; commands validate every number (finite, in range) and report
  errors rather than panicking.
* Parameter keys such as `path`, `file`, `folder` or `*_path` are checked by the host before a command runs, so a
  command cannot be tricked into touching a file outside the roots.

## Testing

* `cargo test -p anicraft-cli --test mcp` runs the real binary over stdio: handshake, tool list, building a document
  with the drawing tools, RTL text, keyframes, a classic tween, rendering and decoding PNGs, a GIF export, saving a
  `.fla` and reopening it.
* `DISPLAY=:1 ANICRAFT_GUI_TESTS=1 ANICRAFT_SHOT_DIR=/some/dir cargo test -p anicraft-cli --test bridge` (after
  `cargo build -p anicraft`) starts the window, drives it through the bridge and saves screenshots of the window and of
  each panel.

## Known limits

* A forced text direction is not stored in XFL; `auto` resolves Hebrew and Arabic runs right-to-left from their letters.
* Sounds can be attached (`timeline_set_sound`) but not imported by MCP yet; sounds already in the library work.
* Saving writes `.fla`; use `anicraft-cli convert` for XFL folders.
