# gradients (200x200, white page)

Verdict (rendered, read, asserted): focal radial (A), sRGB vs linear-RGB bands (B), repeat with a rotated matrix (C) and
straight-alpha interpolation (D) all match the numbers below in anim-render and in Ruffle. No bug.

Aim: focal radial gradients, linear-RGB interpolation, repeat spread with a rotated and stretched gradient matrix, and
straight (non-premultiplied) alpha interpolation.

Cell A (0..100, 0..100): radial, centre (50,50), radius 40, focal 0.9 (focus at x=86), red (t=0) to blue (t=1),
pad, sRGB interpolation. Colour at t is (255(1-t), 0, 255t). For a point on the x axis left of the focus t = distance
from the focus / (1.9 * 40 = 76).
- (85,50): left of the focus by 0.5 px, t = 0.0066 -> (253,0,2).
- (50,50): t = 36/76 = 0.474 -> (134,0,121).
- (11,50): t = 74.5/76 -> (5,0,250).
- (95,50): outside the circle on the focus side, pad -> (0,0,255) (t=1).
- the ramp is not symmetric around the centre: (30,50) is 55.5 left of the focus, t=0.730 -> (69,0,186); (70,50) is 15.5
  left of the focus, t=0.204 -> (203,0,52).

Cell B (100..200, 0..100): black to white over x 110..190, pad. Upper band y<50 interpolates in linear RGB, lower band
y>=50 in sRGB.
- sRGB band: (150,75) -> 128 grey, (130,75) -> 64, (170,75) -> 191 (+-4).
- linear band (sRGB-encoded linear light): (150,25) -> 188, (130,25) -> 137, (170,25) -> 225 (+-8).

Cell C (0..100, 100..200): linear gradient, stops red@0, green@0.2, blue@1.0, repeat spread. Gradient x axis mapped to
screen +y (rotated 90 degrees) with half-extent 25 px about (50,150): t=0 at y=125, t=1 at y=175, period 50 px along y,
no dependence on x.
- (20,130) and (80,130): t=0.1 -> (128,128,0).
- (50,135): t=0.2 -> (0,255,0).
- (50,155): t=0.6 -> (0,128,128).
- (50,180): t=0.1 (repeat) -> (128,128,0).
- (50,120): t=0.9 -> (0,32,223).

Cell D (100..200, 100..200): left half (x<150) black, right half white backdrop, over it a rect with a horizontal
gradient x 110..190 from (0,0,255, alpha 0) to (255,0,0, alpha 255). Straight alpha interpolation:
- (140,150): t=0.375 colour (96,0,159) alpha 0.375 over black -> (36,0,60).
- (160,150): t=0.625 colour (159,0,96) alpha 0.625 over white -> (195,96,156). (A premultiplied interpolation would
  give (255,96,96).)
