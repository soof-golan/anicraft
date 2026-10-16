# color_fx

Target: every kind of instance colour effect, nested alpha, and visibility. Eight 20x20 blue (#0000FF) squares on a white
stage of 100x100; the square at column i, row j covers x 30i..30i+20, y 30j..30j+20. Probe at the centre (+10,+10).

Formulas (channels 0..255): tint with amount m and colour t: c*(1-m) + t*m. Brightness b >= 0: c + (255-c)*b;
b < 0: c*(1+b). Advanced: c*mul + offset. Alpha is composed over the white stage: out = c*a + 255*(1-a).

1. (10,10) tint 50% red over blue: (127.5, 0, 127.5) -> (128,0,128) +-2.
2. (40,10) brightness +50%: (127.5, 127.5, 255) -> (128,128,255) +-2.
3. (70,10) brightness -50%: (0, 0, 127.5) -> (0,0,128) +-2.
4. (10,40) alpha 50%: blue over white (127.5,127.5,255) -> (128,128,255) +-2.
5. (40,40) red mul 0, blue mul 0.5, red offset 255: (255, 0, 127.5) -> (255,0,128) +-2.
6. (70,40) symbol Pair (containing a square with alpha 50%) with alpha 50% on the instance: alphas multiply to 25%:
   (191.25, 191.25, 255) -> (191,191,255) +-2.
7. (10,70) tint 100% yellow: (255,255,0) exactly.
8. (40,70) alpha 0: white (255,255,255).
9. (70,70) `isVisible="false"`: white.
10. Gaps between squares (e.g. (25,25), (55,55)) are white.

Verdicts (`color_fx_x6.png`): points 1-10 hold, including nested alpha 25% and the hidden and alpha-0 squares.
