# blend

Target: blend modes of symbol instances. Backdrop #C86432 = (200,100,50) over the whole 100x100 stage; nine 20x20 source
squares #6496C8 = (100,150,200), opaque, at (30i, 30j), each an instance with its own `blendMode`. Probe at the centre
(30i+10, 30j+10); tolerance 3 per channel. Gaps keep the backdrop (200,100,50).

Per channel with b = backdrop, s = source (0..255): multiply b*s/255; screen 255-(255-b)(255-s)/255; difference |b-s|;
lighten max; darken min; add min(255,b+s); subtract max(0,b-s) (backdrop minus source);
hardlight s<=127.5 ? mult(b,2s) : screen(b,2s-255); overlay b<=127.5 ? mult(2b,s) : screen(2b-255,s).

| cell   | mode       | expected          |
|--------|------------|-------------------|
| (0,0)  | multiply   | (78, 59, 39)      |
| (1,0)  | screen     | (222, 191, 211)   |
| (2,0)  | difference | (100, 50, 150)    |
| (0,1)  | lighten    | (200, 150, 200)   |
| (1,1)  | add        | (255, 250, 250)   |
| (2,1)  | subtract   | (100, 0, 0)       |
| (0,2)  | darken     | (100, 100, 50)    |
| (1,2)  | overlay    | (188, 118, 78)    |
| (2,2)  | hardlight  | (157, 127, 167)   |

Verdicts (`blend_x6.png`): all nine cells match the formulas within 3 levels; gaps keep the backdrop.
