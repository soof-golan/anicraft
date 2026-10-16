# joins_caps_dashes (200x200, white page, dark = (30,30,30))

Verdict (rendered, crops at 6x read). anim-render matches every point. Ruffle matches except the 20 degree chevron: it
draws the full miter spike (the tip runs up to y=0 instead of being bevelled flat at y=14), i.e. Ruffle ignores the
miter limit of LINESTYLE2. The SWF we write carries the limit (8.8 fixed point), so this is a player deviation,
not an export bug; the test only asserts the bevel for anim-render and a loose probe for Ruffle.

Aim: miter limit, caps, dashes that start exactly on corners and across the closing segment, hairpins.

Cell A (0..100, 0..100), width 10, miter join, limit 3, butt caps:
- 60 degree chevron apex (25,20), arms to (1.91,60) and (48.09,60): ratio 1/sin30 = 2 <= 3, the miter stays; the tip
  reaches y=10: (25,12) dark, (25,8) white.
- 20 degree chevron apex (75,15), arms to (62.7,85) and (87.3,85): ratio 5.76 > 3, so the join is bevelled; the top
  is flat at y = 14.13: (75,13) white, (75,16) dark.

Cell B (100..200, 0..100), width 12, lines from x=115 to x=155 at y=20 (no cap), y=50 (round), y=80 (square):
- y=20: (114,20) white, (116,20) dark. y=50: (110,50) dark, (108,50) white, corner (110,45) light (outside the arc).
- y=80: (110,80) dark and the corner (110,75) dark, (108,80) white.

Cell C (0..100, 100..200), closed rectangle (10,110)-(90,190), width 4, butt, dashes [10, 6]; the dash phase starts at
(10,110). The top edge is 5 periods long, so a new dash starts exactly at the corner (90,110).
- top edge: (15,110) dark, (23,110) white, (31,110) dark, (39,110) white.
- right edge: dash on y 110..120: (90,115) dark, (90,123) white, (90,130) dark; (91,109) white (butt end).
- bottom edge position 160..240; left edge dash at position 240 starts at (10,190): (10,185) dark, (10,177) white.
- the last dash before the path closes covers y 126..116 on the left edge and the gap covers 116..110, so (10,113) white
  and (10,120) dark.

Cell D (100..200, 100..200), width 10: two hairpins that return 2 px away.
- (110,125) -> (180,125) -> (110,127) round join: the turn is a half disc, (184,126) dark, (186,126) white.
- (110,175) -> (180,175) -> (110,177), miter limit 4: bevelled flat at x ~ 180: (178,176) dark, (182,176) white.
