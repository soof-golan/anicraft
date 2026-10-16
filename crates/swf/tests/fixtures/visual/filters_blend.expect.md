# filters_blend (200x200, white page)

Verdict (rendered, read, asserted): horizontal-only blur stays crisp vertically, knockout shadow, group-level multiply
(overlap shows only the top child) and difference with alpha 0.5 all hold in anim-render and Ruffle. No bug.

Aim: directional filters, knockout, group blend flattening and blend combined with alpha.

Cell A (0..100, 0..100): red square (30,30)-(70,70), Blur x=8 y=0 quality 3. Only horizontal blur:
- top edge stays crisp: (50,29) white, (50,31) red (255,0,0).
- the left edge is soft and symmetric around x=30: green channel g(x) at (x,50) satisfies g(30-d)+g(29+d) ~ 255 (+-30)
  for d=1..6; (20,50) near white (g>200), (40,50) pure red (g<30), (30,50) mid (g between 60 and 200).
- the blur does not leak vertically: (50,26) white.

Cell B (100..200, 0..100): blue square (130,30)-(170,70), drop shadow black, angle 45, distance 10, blur 0, strength 1,
knockout. Shadow offset (+7.07,+7.07). Knockout removes the object itself:
- (150,50) white (knocked out), (125,50) white, (172,72) black (shadow outside the object), (150,75) black? (the shadow
  covers y up to 77.07, x 137..177: yes black), (150,80) white.

Cell C (0..100, 100..200): yellow background rect; over it a group with blend Multiply containing cyan (10,110)-(60,160) and
magenta (40,140)-(90,190), magenta on top. The group is composited normally inside, then multiplied:
- (20,120) cyan*yellow = (0,255,0); (80,180) magenta*yellow = (255,0,0); (50,150) overlap shows magenta only ->
  (255,0,0) (not cyan*magenta*yellow = black); (95,195) yellow (255,255,0).

Cell D (100..200, 100..200): background rect (200,50,50); group with blend Difference and colour multiplier alpha 0.5
containing a white square (120,120)-(180,180). Difference of white over (200,50,50) = (55,205,205), half mixed back:
(128,128,128). Outside the square (110,110) stays (200,50,50).
