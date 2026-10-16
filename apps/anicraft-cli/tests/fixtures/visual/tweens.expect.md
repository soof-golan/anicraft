# tweens

Target: classic tween rotation across the 0/360 wrap, a clockwise tween with an extra turn, and a shape tween.
Stage 100x100, 11 frames (keyframes at 0 and 10), linear. Probes use 0-based frame numbers (`render --frame` is that + 1).
Bar: blue 40x6 px bar, 0..40 along its x axis, centred on its y axis (y -3..3).

Layer `wrap`: pivot (30,50), keyframe 0 rotated 350 degrees (= -10, counter-clockwise on screen), keyframe 10 rotated +10 degrees.
Auto rotation takes the short way: -10 -> +10, through 0, never through 180. Rotation at frame f: -10 + 2f degrees.
- f=0: tip at (69.4, 43.1). (65,44) blue, (50,47) blue, (65,50) white (6 px off the axis).
- f=5: rotation 0: horizontal bar X 30..70, Y 47..53: (60,50) blue, (60,44) white, (60,56) white, (20,50) white.
  (The long way round would give 180 degrees here: (60,50) white and (10,50) blue.)
- f=10: rotation +10: tip at (69.4, 56.9). (65,56) blue, (65,50) white.

Layer `cw`: pivot (50,20), `motionTweenRotate="clockwise"` with 1 extra turn, 0 -> 90 degrees: total 450 degrees.
Rotation at frame f: 45 f degrees. At f=0 horizontal to the right, at f=5 225 degrees (up and to the left), at f=10 90 degrees (down).
- f=0: (70,20) blue, (50,40) white.
- f=5: bar from (50,20) towards (30,0): (40,10) blue, (60,30) white (where an un-turned 45 degree bar would be), (70,20) white.
- f=10: (50,40) blue, (70,20) white.

Layer `morph`: shape tween of a green (#00C000) bar, keyframe 0 X 10..30, keyframe 10 X 10..90, Y 80..90.
Right edge at frame f is at 30 + 6 f.
- f=0: (20,85) green, (40,85) white.
- f=5: right edge at 60: (50,85) green, (70,85) white, (10.5..) left edge fixed: (12,85) green, (8,85) white.
- f=10: (80,85) green; (92,85) white.

Verdicts (`tweens_f0/f5/f10_x6.png`): the classic tweens hold: the wrap layer takes the short way (f5 is horizontal),
the clockwise layer turns 450 degrees (f5 points up-left, f10 down). Shape tween: edges and colour hold, but the
prediction of crisp rectangular corners was too strict. At f5 the 16x crop (`tweens_f5_crop.png`) shows the corners
chamfered by about 3 px and the ends bulging 0.5 px past x=10 and x=60. That is the documented behaviour of the
default Distributive blend in anim-morph ("corners may soften"), which resamples by arc length; Animate's own
Distributive blend also softens. Not a reader bug; the probes sit away from the corners and `blend="angular"` would
keep them.
