# text_rtl

Target: bidirectional ordering of coloured runs in static text fields. Stage 200x110, white, three fields of 22 px text.
Colours mark runs: red = Hebrew "שלום", blue = Hebrew "עולם" or "!", green = Latin "abc". "Centroid" means the mean x of the
pixels that clearly belong to a colour (red: r>200,g<80,b<80; blue: b>200,r<80,g<80; green: g>120,r<80,b<80).

The paragraph direction is that of the first strong character (Unicode bidi, rule P2/P3). Runs are logical order.

1. Field 1 (y 5..35): logical `שלום`(red) ` עולם`(blue) ` abc`(green). Hebrew first, so the paragraph is right-to-left: the
   first run goes to the right. Visual left to right: green (abc), blue (עולם), red (שלום).
   centroid(green) < centroid(blue) < centroid(red); the three clusters do not overlap horizontally.
2. Field 2 (y 40..70): logical `abc `(green) `שלום`(red) `!`(blue). Latin first, so the paragraph is left-to-right; the
   Hebrew run is reversed inside, and the trailing neutral `!` takes the paragraph direction: it goes to the RIGHT of the
   Hebrew. Visual left to right: green, red, blue. centroid(green) < centroid(red) < centroid(blue).
3. Field 3 (y 75..105): logical `שלום`(red) `!`(blue). Hebrew first, RTL paragraph: the `!` (neutral at the end of the
   paragraph, which takes the paragraph direction) goes to the LEFT of the Hebrew: centroid(blue) < centroid(red).
4. Every field shows ink of every colour it has (at least 20 pixels per colour), and no ink lies outside its own y band
   (fields are 5 px apart at least: no colour pixels at y in 36..39 or 71..74 for the text of the other fields is not
   asserted; only that the red cluster of field 1 is within y 0..40).
5. Fonts: the Hebrew glyphs need a font with Hebrew coverage; the shaper must fall back to one if "Arial" is missing
   (no empty boxes: the red pixel count of field 1 is at least 100).

Verdicts (`text_rtl_x6.png`): points 1-5 hold. Field 1 reads green, blue, red left to right; field 2 green, red, then
`!` right of the Hebrew; field 3 has `!` at the far left. The Hebrew glyphs are real glyphs, not boxes.
