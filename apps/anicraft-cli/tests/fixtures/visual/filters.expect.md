# filters

Target: blur extents (no hard crop of the blurred halo), symmetry, drop shadow geometry and `hideObject`. Stage 100x100.

Instances of 10x10 squares: Black (#000000) at (20,20) with blur 10x10 quality 2; Red (#FF0000) at (20,60) with a
hard (blur 0) black drop shadow, distance 10, angle 45 degrees; Red at (60,60) the same but `hideObject`; Black at
(70,15) with a horizontal-only blur of 30 (quality 1).

1. Blur halo. The square covers X 20..30, Y 20..30 (centre 25,25). A blur of 10 with 2 passes spreads ink about 10 px
   beyond each edge and never more than that.
   - Symmetry: the pixels 5 px outside each edge at the middle of the side, (15,25) (35,25) (25,15) (25,35), have the
     same value within 4 levels, and that value is neither white nor black (between 120 and 250).
   - 15 px outside the edge the stage is clean: (5,25) (45,25) (25,5) (25,45) are >= 253 in every channel.
   - The centre is lighter than pure black (blur spreads the ink out): between 40 and 170 in every channel.
   - No hard crop: along the row y=25 the red channel never decreases going from x=25 out to x=45 by more than a smooth
     step: the difference between neighbouring pixels stays below 60.
2. Drop shadow (offset (7.07, 7.07)): the shadow covers X 27.07..37.07, Y 67.07..77.07, the red square X 20..30, Y 60..70
   is drawn on top.
   (22,62) red (255,0,0); (34,74) black (<= 20 in every channel); (28,72) black; (34,65) white (above the shadow);
   (24,75) white (left of the shadow); (45,75) white.
3. `hideObject`: the object itself is hidden, only the shadow remains: the square is X 60..70, Y 60..70, shadow X 67.07..77.07,
   Y 67.07..77.07: (62,62) white, (72,72) black, (65,65) white (inside the object but outside the shadow).
4. Horizontal blur 30 on a 10x10 black square at (70,15)..(80,25): one box pass of 30 spreads 15 px each side
   horizontally and nothing vertically. Row y=20: (62,20) is not white (< 250) and (96,20)... off the right side is
   white at x >= 99: (99,20) >= 250. Vertically sharp: (75,13) white, (75,28) white. The ink is symmetric about x=75:
   |value(75-d,20) - value(75+d,20)| <= 4 for d = 10.

Verdicts (`filters_x6.png`): the prediction was wrong in one probe, the rest hold. A pixel x covers x..x+1, so the
mirror of (35,25) about the square's centre 25 is (14,25), not (15,25); with (15,25) the values differ by 10
(226 vs 236) purely from the half-pixel. Probes now use (14,25), (25,14) and, for the horizontal blur, (65,20) vs
(84,20); the blur is symmetric within 4 levels. Halo ends before 15 px, no hard crop, shadow offset and `hideObject`
as predicted; the faint grey fringe on the shadow's far edges is the 0.07 px fractional offset.
