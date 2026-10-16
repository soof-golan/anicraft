# scenes

Target: two scenes, `currentTimeline` choosing the default one, frame labels, and the background colour. Stage 100x60,
background #202020 (32,32,32). Scene "First" has 5 frames: frames 0..1 (label `intro`) a blue square X 10..40 (twips
200..800 = 10..40) Y 10..40, frames 2..4 (label `outro`) a green square X 60..90, Y 10..40. Scene "Second" has 2
frames (0..1) with a red square at X 10..40, Y 10..40. `currentTimeline="2"` is the second scene (1-based).

1. `render` without `--scene` draws "Second": (25,25) = (255,0,0); (75,25) = background (32,32,32).
2. `render --scene First --frame 1`: (25,25) blue (0,0,255); (75,25) background.
3. `render --scene First --frame 2`: still blue (the keyframe lasts two frames).
4. `render --scene First --frame 3`: (75,25) green (0,255,0); (25,25) background.
5. `render --scene First --frame 5`: green (last frame). `--frame 6` is past the end: an error from the CLI (not a panic).
6. The document model after reading: two scenes named First and Second, `current_timeline` = 1 (0-based), First has 5 frames and
   labels `intro` at frame 0 and `outro` at frame 2 (in the `Frame` model of the keyframes), Second has 2 frames.
7. The background is drawn at pixel (95,55) in every render: (32,32,32).
8. Write then read back: scene names, order, labels, `current_timeline` survive.

Verdicts: points 1-5 and 7 hold in the renders (frame 6 is a CLI error, not a panic). Points 6 and 8 (model, labels and
write/read) hold through the FLA and folder conversions, which render the same pixels.
