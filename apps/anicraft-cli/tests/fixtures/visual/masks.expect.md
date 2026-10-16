# masks

Target: mask layers (one with a hole), guide layers, hidden layers, an unmasked layer below a masked one, and a mask
inside a scaled symbol. Stage 100x100, white.

Layers top to bottom: `clipinst` (instance of Clip, scale 2, translate 60,5), `mask` (mask: 40x40 square X 20..60,
Y 20..60 with a square hole X 30..50, Y 30..50, wound the other way), `masked` (red X 0..100, Y 40..80, child of the
mask), `guide` (green, whole stage, guide layer), `hidden` (blue X 5..25, Y 85..95, `visible="false"`), `bottom`
(yellow X 70..90, Y 85..95, not masked).

Clip: mask square 0..20 over a red square -10..30; at scale 2 the visible part is X 60..100, Y 5..45.

1. Red visible region = (mask minus hole) intersect masked rect = X 20..60, Y 40..60 minus the hole part X 30..50, Y 40..50.
   (25,45) red, (55,45) red, (40,55) red, (25,58) red.
2. The hole shows the background: (40,45) white; the mask itself is never painted: (25,25) white, (40,35) white.
3. Outside the mask the masked red is cut: (65,50) white, (25,65) white, (10,50) white.
4. The guide layer is not drawn: (5,5) white, (50,95) white.
5. The hidden layer is not drawn: (15,90) white.
6. The layer below the mask is unmasked and visible: (80,90) yellow (255,255,0).
7. Clip symbol: (80,25) red, (65,10) red, (98,43) red; (55,25) white (red square would reach X 40), (80,50) white,
   (80,3) white, (102..) off stage.
8. Edges are crisp: red region right edge between (59,45) red and (61,45) white.

Verdicts (`masks_x6.png`): points 1-8 hold. The hole shows white, the notch is X 30..50 Y 40..50, guide and hidden
layers are absent, the unmasked yellow bar is drawn, the clipped symbol shows red only at X 60..100 Y 5..45, edges are
crisp at x=60.
