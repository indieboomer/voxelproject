"""Build the project's original 5x7 Voxel Rune font (requires fonttools)."""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

# Five-bit rows, top to bottom. Code editors retain egui's regular monospace font.
ROWS = {
    'A':'0E 11 11 1F 11 11 11', 'B':'1E 11 11 1E 11 11 1E',
    'C':'0F 10 10 10 10 10 0F', 'D':'1E 11 11 11 11 11 1E',
    'E':'1F 10 10 1E 10 10 1F', 'F':'1F 10 10 1E 10 10 10',
    'G':'0F 10 10 17 11 11 0F', 'H':'11 11 11 1F 11 11 11',
    'I':'0E 04 04 04 04 04 0E', 'J':'07 02 02 02 12 12 0C',
    'K':'11 12 14 18 14 12 11', 'L':'10 10 10 10 10 10 1F',
    'M':'11 1B 15 15 11 11 11', 'N':'11 19 15 13 11 11 11',
    'O':'0E 11 11 11 11 11 0E', 'P':'1E 11 11 1E 10 10 10',
    'Q':'0E 11 11 11 15 12 0D', 'R':'1E 11 11 1E 14 12 11',
    'S':'0F 10 10 0E 01 01 1E', 'T':'1F 04 04 04 04 04 04',
    'U':'11 11 11 11 11 11 0E', 'V':'11 11 11 11 11 0A 04',
    'W':'11 11 11 15 15 1B 11', 'X':'11 11 0A 04 0A 11 11',
    'Y':'11 11 0A 04 04 04 04', 'Z':'1F 01 02 04 08 10 1F',
    '0':'0E 11 13 15 19 11 0E', '1':'04 0C 04 04 04 04 0E',
    '2':'0E 11 01 02 04 08 1F', '3':'1E 01 01 0E 01 01 1E',
    '4':'02 06 0A 12 1F 02 02', '5':'1F 10 10 1E 01 01 1E',
    '6':'0E 10 10 1E 11 11 0E', '7':'1F 01 02 04 08 08 08',
    '8':'0E 11 11 0E 11 11 0E', '9':'0E 11 11 0F 01 01 0E',
    ' ':'00 00 00 00 00 00 00', '.':'00 00 00 00 00 0C 0C',
    ',':'00 00 00 00 0C 0C 08', ':':'00 0C 0C 00 0C 0C 00',
    ';':'00 0C 0C 00 0C 0C 08', '!':'04 04 04 04 04 00 04',
    '?':'0E 11 01 02 04 00 04', '-':'00 00 00 1F 00 00 00',
    '_':'00 00 00 00 00 00 1F', '+':'00 04 04 1F 04 04 00',
    '=':'00 00 1F 00 1F 00 00', '/':'01 02 02 04 08 08 10',
    '\\':'10 08 08 04 02 02 01', '(':'02 04 08 08 08 04 02',
    ')':'08 04 02 02 02 04 08', '[':'0E 08 08 08 08 08 0E',
    ']':'0E 02 02 02 02 02 0E', '{':'03 04 04 08 04 04 03',
    '}':'18 04 04 02 04 04 18', '<':'01 02 04 08 04 02 01',
    '>':'10 08 04 02 04 08 10', '|':'04 04 04 04 04 04 04',
    '"':'0A 0A 0A 00 00 00 00', "'":'04 04 08 00 00 00 00',
    '`':'08 04 02 00 00 00 00', '~':'00 00 09 16 00 00 00',
    '*':'00 15 0E 1F 0E 15 00', '#':'0A 0A 1F 0A 1F 0A 0A',
    '$':'04 0F 14 0E 05 1E 04', '%':'19 1A 02 04 08 0B 13',
    '&':'0C 12 14 08 15 12 0D', '@':'0E 11 17 15 17 10 0F',
    '^':'04 0A 11 00 00 00 00',
}

def build():
    rows = dict(ROWS)
    rows.update({
        '\u2192':'00 04 02 1F 02 04 00', '\u2190':'00 04 08 1F 08 04 00',
        '\u2014':'00 00 00 1F 00 00 00', '\u2013':'00 00 00 0E 00 00 00',
        '\u00D7':'00 00 11 0A 04 0A 11', '\u2026':'00 00 00 00 00 00 15',
        'a':'00 00 0E 01 0F 11 0F', 'b':'10 10 1E 11 11 11 1E',
        'c':'00 00 0F 10 10 10 0F', 'd':'01 01 0F 11 11 11 0F',
        'e':'00 00 0E 11 1F 10 0F', 'f':'06 08 08 1E 08 08 08',
        'g':'00 0F 11 11 0F 01 0E', 'h':'10 10 1E 11 11 11 11',
        'i':'04 00 0C 04 04 04 0E', 'j':'02 00 06 02 02 12 0C',
        'k':'10 10 12 14 18 14 12', 'l':'0C 04 04 04 04 04 0E',
        'm':'00 00 1A 15 15 15 15', 'n':'00 00 1E 11 11 11 11',
        'o':'00 00 0E 11 11 11 0E', 'p':'00 1E 11 11 1E 10 10',
        'q':'00 0F 11 11 0F 01 01', 'r':'00 00 16 19 10 10 10',
        's':'00 00 0F 10 0E 01 1E', 't':'08 08 1E 08 08 09 06',
        'u':'00 00 11 11 11 11 0F', 'v':'00 00 11 11 11 0A 04',
        'w':'00 00 11 11 15 15 0A', 'x':'00 00 11 0A 04 0A 11',
        'y':'00 11 11 11 0F 01 0E', 'z':'00 00 1F 02 04 08 1F',
    })
    fb = FontBuilder(1000, isTTF=True)
    names = ['.notdef'] + [f'uni{ord(c):04X}' for c in sorted(rows)]
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({ord(c): f'uni{ord(c):04X}' for c in rows})
    glyphs, metrics = {}, {}
    for char, name in [(None, '.notdef')] + [(c, f'uni{ord(c):04X}') for c in sorted(rows)]:
        pen = TTGlyphPen(None)
        for row, value in enumerate(rows.get(char, '1F 11 15 15 15 11 1F').split()):
            for col in range(5):
                if int(value, 16) & (1 << (4-col)):
                    x, y = col*100, (6-row)*100
                    pen.moveTo((x,y)); pen.lineTo((x,y+100))
                    pen.lineTo((x+100,y+100)); pen.lineTo((x+100,y)); pen.closePath()
        glyphs[name] = pen.glyph()
        metrics[name] = (600, 0)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({'familyName':'Voxel Rune','styleName':'Regular',
        'uniqueFontIdentifier':'VoxelRune-Regular-1','fullName':'Voxel Rune Regular',
        'psName':'VoxelRune-Regular','version':'Version 1.0'})
    fb.setupOS2(sTypoAscender=800,sTypoDescender=-200,usWinAscent=800,usWinDescent=200)
    fb.setupPost(); fb.setupMaxp()
    fb.font['head'].created = fb.font['head'].modified = 3860000000
    path = Path(__file__).resolve().parents[1] / 'assets/fonts/voxel-rune.ttf'
    fb.save(path)
    print(path)

if __name__ == '__main__':
    build()
