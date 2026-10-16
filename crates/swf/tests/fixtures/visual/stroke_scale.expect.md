# stroke_scale (400x200, white page)

Verdict. BUG FOUND AND FIXED: the exporter set the LINESTYLE2 bits for Horizontal/Vertical the wrong way round
(Horizontal wrote NoVScale). Predicted Horizontal row 1 vertical bar 4 px wide; the exported SWF said the opposite.
Now NoHScale for Horizontal, NoVScale for Vertical; the bits are asserted by parsing the SWF back, and anim-render's
pixels match the table. Ruffle itself does not implement NoHScale/NoVScale (it draws all four like Normal), so the
Ruffle picture cannot be used for this fixture; tests/visual.rs::player_follows_the_stroke_scale_bits documents that.

Aim: the four stroke scale modes under non-uniform scaling. Red, butt caps, local width 4. Four cells per row (Normal,
Horizontal, Vertical, None) at x = 0,100,200,300.

SWF/Flash rule: noHScale ("Horizontal") keeps the thickness from following horizontal scaling; noVScale ("Vertical")
keeps it from following vertical scaling; None follows neither. The anim-render implementation of `Horizontal` (see
its `stroke.rs` docs) is the same rule, so Ruffle and anim-render must agree.

Row 1: group scale (4,1): a vertical bar at screen x=50 (cell-relative) and a horizontal bar at y=50.
Thickness of the vertical bar measured along x, of the horizontal bar along y:

| mode       | vertical bar width (x) | horizontal bar height (y) |
|------------|------------------------|---------------------------|
| Normal     | 16 (x 42..58)          | 4 (y 48..52)              |
| Horizontal | 4 (x 48..52)           | 4                         |
| Vertical   | 16                     | 4                         |
| None       | 4                      | 4                         |

Row 2: group scale (1,4), cells at y offset 100: the horizontal bar is at screen y=150, the vertical one at x=50.

| mode       | vertical bar width (x) | horizontal bar height (y) |
|------------|------------------------|---------------------------|
| Normal     | 4                      | 16 (y 142..158)           |
| Horizontal | 4                      | 16                        |
| Vertical   | 4                      | 4 (y 148..152)            |
| None       | 4                      | 4                         |

Probe: count red pixels (red channel > 200 and green < 60) along y=30 (row 1, crossing the vertical bar only) and along
x=20 of each cell in row 2 (crossing the horizontal bar only, local x=20 is inside the horizontal bar's span).
Edges are on pixel boundaries, so counts are exact with +-1 tolerance for antialiasing.
