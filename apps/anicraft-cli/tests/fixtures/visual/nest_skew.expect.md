# nest_skew

Target: three nested graphic symbols with a skew, a mirror and off-origin transformation points. A symbol instance
matrix `[a b c d tx ty]` maps `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`; the transformation point is only the pivot
of the transform tool and must not move anything. Stage 100x100.

Cell: blue square 0..10 x 0..10, red marker 0..2 x 0..2 at its top left corner (the marker layer is on top).
Inner holds Cell with `a=-1 tx=20` (x' = 20 - x). Outer holds Inner with `a=1 b=0 c=1 d=1 tx=10 ty=10`
(x' = x + y + 10, y' = y + 10). The stage instance has the identity matrix and the transformation point (40,-30).

Composite of a Cell point (x, y): X = (20 - x) + y + 10 = 30 - x + y, Y = y + 10.

1. The square becomes a parallelogram with corners (30,10) (20,10) (40,20) and (30,20)... precisely:
   (0,0) -> (30,10); (10,0) -> (20,10); (10,10) -> (30,20); (0,10) -> (40,20).
   Horizontal rows are 10 px wide and shift right 1 px per row.
2. Row y=15 (Cell y=5) spans X = 25..35: (24,15) white, (26,15) blue, (34,15) blue, (36,15) white.
3. Row y=19 (Cell y=9) spans X = 29..39: (28,19) white, (30,19) blue, (38,19) blue, (40,19) white.
4. Row y=11 (Cell y=1) spans X = 21..31; the marker covers Cell x 0..2, i.e. X 29..31: (30,11) red, (22,11) blue.
   The marker is at the TOP RIGHT of the parallelogram (the mirror moved it from the left).
5. Outside: (35,12) white (right of the top row, which ends at x = 31 at y = 11), (15,15) white, (30,25) white.
6. The transformation points (40,-30), (50,50), (5,5) leave all of this untouched.

Verdicts (rendered at 1x and inspected at 6x, `nest_skew_x6.png`): points 1-6 all hold. Parallelogram top edge X 20..30,
bottom edge X 30..40, red marker at the top right, rows exactly as predicted, nothing outside. Asserted in
`tests/visual_xfl.rs`.
