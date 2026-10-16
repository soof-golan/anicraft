# multiframe_diff (100x100, page white, six frames)

Verdict (all six frames rendered by both, f0 and f1 read): every frame point holds, placement resets stick, character
replacement and removal work. No bug.

Aim: the frame differ must reset properties on Move placements and replace characters when they change.

Always: depth 1 is a yellow rect (255,255,0) over the stage, group G (cyan child rect (0,0)-(40,40)) at depth 2, a
red rect (70,70)-(90,90) at depth 3.

- f0: G translate (20,20), blend Multiply, colour mul alpha 0.5, Blur 4x4 q1. Interior (40,40): multiply cyan on yellow
  = (0,255,0), half mixed -> (128,255,0) (+-8). (10,10) yellow. red at (80,80).
- f1: G blend Normal, no filters, identity colour: G fully cyan and crisp: (40,40) (0,255,255), (19,40) yellow,
  (20,40) cyan, (59,40) cyan, (60,40) yellow. Placement resets must stick.
- f2: G moved to translate (50,20) only (matrix change), blend Subtract: yellow - cyan = (255,0,0). (70,40) = (255,0,0);
  the old place (30,40) is yellow. (The identity colour from f1 is kept.)
- f3: red rect removed, a green rect (0,200,0) (10,70)-(30,90) added at depth 3 (character change). G back at (20,20),
  Normal. (80,80) yellow, (20,80) green, (40,40) cyan.
- f4: G gets a mask rect (0,0)-(20,40) (left half): (30,40) cyan, (50,40) yellow. Green still at (20,80).
- f5: no nodes at all: everything removed, (50,50) white (the page), (30,40) white.

Cross-check: anim-render of each list agrees with Ruffle at the probes (blur tolerance 12).
