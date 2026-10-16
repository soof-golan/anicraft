# fill_sides (200x200, white page)

Verdict (rendered, both PNGs read and probed; asserted in tests/visual.rs): every point holds in anim-render and
Ruffle (cells A-D, winding, even-odd, integer edges). The two renderings differ on under 1% of the pixels. No bug.

Aim: fill0/fill1 side choice and winding rules survive the export. Orientation of a contour must never change
what a single solid fill covers; only the winding rule and contour nesting decide holes.

Cell A (0..100, 0..100), red, non-zero:
- a clockwise square (10,10)-(40,40) and a counter-clockwise one (60,10)-(90,40): both filled. (25,25) red, (75,25) red,
  (50,25) white. Integer edges: pixel x=9 white, x=10 red (y likewise).
- a bow-tie (10,55)(45,55)(10,90)(45,90): the two triangles have opposite winding, non-zero still fills both. (27,60) red,
  (27,85) red, (12,72) white, (43,72) white.

Cell B (100..200, 0..100), blue, non-zero, one path of three nested squares: outer CW (105,5)-(195,95), middle CCW
(120,20)-(180,80), inner CW (140,40)-(160,60).
- ring blue: (110,50), (190,50). hole white: (125,50), (170,50). island blue: (150,50).

Cell C (0..100, 100..200): two pentagrams (radius 22, one path each, vertices 144 degrees apart, first at the top),
green, centres (25,150) non-zero and (75,150) even-odd.
- centres: (25,150) green (winding 2 filled), (75,150) white (winding 2 is a hole under even-odd).
- a tip 15 px above the centre (25,135) and (75,135): green in both. A point between two tips (25+20,150-16)... is white
  in both: (45,134) white, (95,134) white.

Cell D (100..200, 100..200), orange, even-odd, two overlapping same-direction squares (110,110)-(160,160) and
(140,140)-(190,190): (125,125) orange, (175,175) orange, (150,150) white (overlap is a hole), (125,175) white,
(175,125) white.

Cross-check: anim-render of the same list must agree with Ruffle on every probe above.
