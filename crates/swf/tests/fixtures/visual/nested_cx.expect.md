# nested_cx (200x200, white page)

Verdict (rendered, read, asserted). BUG FOUND AND FIXED in cell 4: the exporter dropped a fill whose alpha is 0, but a
colour transform with an alpha add above it makes the player draw it (the transform acts on the colour even at alpha 0),
so the red square was missing from the SWF. `is_visible` now keeps alpha-0 fills under a transform that adds alpha.
Cells 1-3 (skew, concatenation without clamping, rotation) held from the start.

Aim: colour transforms concatenated through nested groups (with alpha, add terms, clamping), together with skew and
rotation, plus a transparent fill revealed by an alpha offset.

Cell 1 (0..100, 0..100): red square (0,0)-(40,40) in Inner (mul (0.5,1,1,1), add g +51/255) inside Outer (skew matrix
x' = x + 0.5y + 20, y' = y + 20; mul alpha 0.5). Pixel (255,0,0) -> inner (127.5,51,0) -> outer alpha 0.5 over white
= (191,153,128).
- parallelogram corners (20,20),(60,20),(80,60),(40,60). Inside: (50,40) and (30,25) and (75,55) are (191,153,128);
  outside (white): (22,45), (78,55)... (78,55) maps to local x = 40.5 (just outside), (10,10).

Cell 2 (100..200, 0..100): blue square +-30 about the origin, Inner has mul alpha 0.5, Middle (rotate 45 degrees) has
mul alpha 0.5 and alpha add +0.5, Outer (translate (150,50)) mul alpha 0.5. Concatenated alpha = ((0.5*0.5)+0.5)*0.5 =
0.375. Colour over white (159,159,255).
- inside the diamond: (150,50), (190,50) (|dx|+|dy|=40 < 42.4), (175,65); outside: (180,70) (50 > 42.4) white.

Cell 3 (0..100, 100..200): green (0,128,0) square (20,120)-(80,180) in Inner (mul g 3, add b +0.5) inside Outer (mul
(0.5,0.5,0.5,1)). Flash concatenates the transforms linearly without clamping between levels: g = 128*3*0.5 = 192, b =
(0+127.5)*0.5 = 64. Pixel (0,192,64). (Clamping after the inner level would give g=128.)

Cell 4 (100..200, 100..200): a square (120,120)-(180,180) filled with (255,0,0, alpha 0) inside a group with
colour transform add alpha +1.0. Flash applies the transform to the fill colour even when its alpha is 0, so the square is
opaque red: (150,150) = (255,0,0).
