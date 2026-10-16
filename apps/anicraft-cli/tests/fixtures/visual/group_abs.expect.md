# group_abs

Target: `DOMGroup` whose members carry their own matrix. In real files (observed in how the shapes of one drawing
connect: a leg inside a group has to meet a hoof outside it, both with the same edge coordinates) a member's matrix
is already the member's placement in the parent timeline, and the group's own `<matrix>` is not applied on top of it.
A member without a matrix of its own is placed by the group's matrix. Prediction below follows that rule.

Stage 100x100, white. All rectangles are 20x20 (or 20x10, 10x10) px in their own edge space.

1. Red square (group member, matrix scale 2 + translate 10,10, edges 0..20): covers x 10..50, y 10..50.
   Pixel (30,30) = #FF0000. If the group matrix were stacked on the member matrix it would sit at x 30..110
   (scale 4, translate 30): then (20,20) would be white, so (20,20) = red is the discriminating probe.
2. Blue square (loose shape, same matrix, edges 20..40): covers x 50..90, y 10..50. Pixel (70,30) = #0000FF.
   Red and blue meet at x = 50 with no gap and no overlap: (49,30) red, (50,30) blue.
3. Green bar (group translate 0,60, member without a matrix, edges 20x10): x 0..20, y 60..70. Pixel (10,65) = #00FF00.
   (10,55) and (10,75) white.
4. Orange (outer group translate 50,60, inner group translate 70,60, shape matrix translate 70,60, edges 10x10):
   total placement is the shape's own matrix: x 70..80, y 60..70. Pixel (75,65) = #FF8000; (55,65) white
   (what a stacked 50+70+70 translation would not give) and (125,..) is off stage.
5. Everything else white: (95,95) = #FFFFFF.
6. Write then read the document again: the model of both reads is equal (matrices of group members compare with
   tolerance 1e-9) and the rendering of both is identical.

Verdicts: BEFORE the fix, point 1 failed (the reader stacked the group matrix over the member's own matrix, the red square
landed at scale 4, see `group_abs_before.png`); this is the root cause of the detached hoof and cropped symbol at
frame 1200 of the unicorn sample (a leg in a group, a hoof outside it). Fix: `read_group` re-expresses members that
carry a matrix relative to the group, the writer undoes it, the semantic diff ignores a repeated matrix.
AFTER: points 1-5 hold (`group_abs_x6.png`: red and blue meet at x=50 with no seam, green bar, orange square
at 70..80). Point 6 holds: converting to FLA and to a rewritten folder renders identical pixels
(`rewriting_a_fixture_does_not_change_its_renders`).
