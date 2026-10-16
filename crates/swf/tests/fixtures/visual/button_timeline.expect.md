# button_timeline (100x100 and 120x60, page white)

Verdict (Ruffle frames read and asserted): up state only, record blend Multiply honoured (black), hit area invisible,
stop() holds the main timeline at frame 3 while the clip stops on its own frame, GotoFrame+Play loops as predicted,
sound and button records parse back. No bug.

## Button (100x100)
DefineButton2 placed with matrix translate(10,10) * scale(2). Button space records:
- depth 1, UP|OVER: red square (0,0)-(20,20). Screen (10,10)-(50,50).
- depth 2, UP: cyan square (0,0)-(10,10) with record matrix translate (5,5) and blend Multiply: multiply on red = black.
  Screen (20,20)-(40,40).
- depth 3, OVER only: green square (0,0)-(10,10) at the top-left; depth 4, DOWN only: blue square (10,0)-(20,10);
- depth 5, HIT only: yellow square (0,0)-(40,40) (screen (10,10)-(90,90)).
With no mouse the player shows the UP state only:
- (15,15) red (not green), (45,15) red (not blue), (30,30) black, (45,45) red, (70,70) white (hit area is invisible),
  (55,55) white.
- Ruffle may or may not honour record blend modes; if (30,30) is red the blend is dropped by the player, not by us:
  check the DefineButton2 bytes by parsing them with the `swf` crate (flags per record: up/over/down/hit, blend present).

## Timeline (120x60), six frames
Main timeline: red square 10x10 at y=10 moving +15 px/frame (x = 10, 25, 40, 55, 70, 85); frame index 3 carries a
`stop`. A sprite on depth 2 (blue square at y=40) with three frames x = 10, 30, 50, `stop` on its frame index 1, a
start-sound on main frame 0 (PCM).
- capturing six frames: red x = 10,25,40,55,55,55 (stopped at frame 3); blue x = 10,30,30,30,30,30.
- sound tags do not disturb the picture and parse back as StartSound.

## Goto (120x60)
Main: frame 3 carries `GotoFrame 1; Play`. Red x for the 8 captured frames: 10,25,40,55,25,40,55,25.
