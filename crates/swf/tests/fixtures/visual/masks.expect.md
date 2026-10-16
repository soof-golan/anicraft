# masks (200x200, white page)

Verdict (rendered, read, asserted): even-odd ring hole, alpha-ignoring mask, nested masks, union of two mask shapes and
the unclipped sibling all hold in anim-render and Ruffle. No bug.

Aim: clip layers. Group masks with holes, translucent masks, nested masks, multi-shape masks and unmasked siblings.

Cell A (0..100, 0..100): green rect covering the cell, masked by an even-odd ring (10,10)-(90,90) minus (30,30)-(70,70).
- (20,50) green, (80,80) green, (50,50) white (hole), (5,5) white, (95,50) white.

Cell B (100..200, 0..100): blue rect masked by a black rect (110,10)-(190,90) filled with alpha 0.3. A Flash mask
treats any painted pixel as opaque (alpha is ignored), so (150,50) is full blue (0,0,255), not (178,178,255). (105,50)
white.

Cell C (0..100, 100..200): outer group, mask = circle centre (50,150) r 40; children: inner group (mask = rect
(0,100)-(50,200), child yellow rect over the whole cell) and a magenta rect (60,140)-(80,160).
- (30,150) yellow (inside circle and inner mask); (49,150) yellow, (50,150) white (inner mask edge at x=50).
- (70,150) magenta (outside the inner mask, inside the circle: the outer mask does not clip it away).
- (70,120) white (circle, right half, no magenta); (10,110) white (outside the circle although in the inner mask).
- (75,150+...) magenta rect is not subject to the inner mask: (75,155) magenta.

Cell D (100..200, 100..200): group with mask = two rects (110,110)-(130,130) and (150,150)-(170,170) (union), cyan child
rect over the whole cell. A red square (172,112)-(192,132) follows the masked group as a sibling and must not be
clipped.
- (120,120) cyan, (160,160) cyan, (140,140) white, (120,160) white, (180,120) red, (160,120) white.
