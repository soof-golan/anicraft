# matrices

Target: rotation by 90 degrees, a point reflection (negative scale on both axes) and a mirror with a non-uniform scale,
each with a transformation point far from the origin that must not matter. Stage 100x100.

Arrow: blue bar 0..20 x 0..10 with a red tip 16..20 x 0..10 at the right end.
`x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.

A. `a=0 b=1 c=-1 d=0 tx=50 ty=10` (turn clockwise by 90 degrees): X = 50 - y, Y = 10 + x.
   Bar covers X 40..50, Y 10..30; the tip (x 16..20) is at the bottom: Y 26..30.
   (45,12) blue, (45,24) blue, (45,28) red, (45,31) white, (39,20) white, (51,20) white.
B. `a=-1 d=-1 tx=90 ty=90` (point reflection): X = 90 - x, Y = 90 - y. Bar covers X 70..90, Y 80..90; the tip is on the
   LEFT: X 70..74. (88,85) blue, (80,85) blue, (72,85) red, (91,85) white, (80,79) white.
C. `a=1 d=-2 tx=10 ty=70` (flip and stretch vertically): X = 10 + x, Y = 70 - 2y. Bar covers X 10..30, Y 50..70; the tip is
   at X 26..30. (15,60) blue, (28,60) red, (15,45) white, (15,72) white, (12,52) blue, (12,68) blue.
D. The three transformation points (10,5), (-300,80) and (0,0) change nothing.

Verdicts (`matrices_x6.png`): A, B, C and D all hold: rotated bar upright with the tip at the bottom, point-reflected bar
with the tip on the left, stretched mirror with the tip on the right, transformation points move nothing.
