"""Original terrain pixel art drawn from primitives, never from source images.

Specifications are in data/base_textures.json. Used by build_resources.py.
"""
import hashlib
import math
import random
from PIL import Image, ImageColor, ImageDraw


def ground_tile(name, base, grass=False):
    """Wrap every grain/tuft; duplicate periodic endpoints for matching edges."""
    rng = random.Random(name)
    period = 15
    shades = [[rng.choice([-7, -3, 0, 0, 4, 7]) for _ in range(period)]
              for _ in range(period)]
    for _ in range(30 if grass else 22):
        x, y = rng.randrange(period), rng.randrange(period)
        delta = rng.choice([-15, -10, 10, 15])
        shades[y][x] = delta
        shades[(y-1) % period if grass else y][x if grass else (x+1) % period] = delta
    im = Image.new('RGBA', (16, 16))
    for y in range(16):
        for x in range(16):
            delta = shades[y % period][x % period]
            im.putpixel((x, y), tuple(max(0, min(255, c+delta)) for c in base)+(255,))
    return im


def render(name, spec):
    rng = random.Random(int.from_bytes(hashlib.sha256(name.encode()).digest()[:8], 'little'))
    base = ImageColor.getrgb(spec['color'])
    pattern = spec['pattern']

    def color(delta=0):
        return tuple(max(0, min(255, c + delta)) for c in base) + (255,)

    im = Image.new('RGBA', (16, 16), color())
    d = ImageDraw.Draw(im)
    for y in range(16):
        for x in range(16):
            d.point((x, y), fill=color(rng.choice([-12, -6, 0, 0, 5, 10])))

    if pattern in ('stone', 'bedrock'):
        # Broken, irregular facets rather than masonry courses.
        facets=[[(0,0),(7,0),(5,5),(0,7)],[(8,0),(15,0),(15,6),(10,8),(6,5)],
                [(0,8),(5,6),(10,9),(8,15),(0,15)],[(11,9),(15,7),(15,15),(9,15)]]
        for points in facets:
            delta=rng.randrange(-15,16)
            d.polygon(points,fill=color(delta))
            d.line(points[:2],fill=color(delta+13))
        for points in [[(0,7),(5,5),(7,0)],[(5,5),(10,8),(15,6)],[(10,8),(8,15)]]:
            d.line(points,fill=color(-27 if pattern=='stone' else -40))
        if pattern=='bedrock':
            d.line((1,11,5,10,6,7),fill=color(-36))
            d.line((11,2,10,5,13,7),fill=color(-36))
    elif pattern=='basalt_top':
        for x,y in [(2,2),(11,2),(6,11),(15,12),(-3,12)]:
            d.polygon([(x-4,y-2),(x,y-5),(x+4,y-2),(x+4,y+3),(x,y+5),(x-4,y+3)],fill=color(-25),outline=color(-38))
            d.polygon([(x-3,y-1),(x,y-3),(x+2,y-1),(x+2,y+2),(x,y+3),(x-3,y+2)],fill=color(rng.randrange(-3,18)))
    elif pattern in ('cobble', 'bricks'):
        rows = [0, 5, 10, 16] if pattern != 'bricks' else [0, 4, 8, 12, 16]
        for row, (y, bottom) in enumerate(zip(rows, rows[1:])):
            widths = [0, 5, 11, 16] if row % 2 == 0 else [0, 3, 9, 16]
            for x, right in zip(widths, widths[1:]):
                delta = rng.randrange(-16, 12)
                d.rectangle((x, y, right-1, bottom-1), fill=color(delta))
                if pattern=='cobble':
                    d.point((x,y),fill=color(-32))
                    d.point((right-2,bottom-2),fill=color(-20))
                d.line((x, bottom-1, right-1, bottom-1), fill=color(-32))
                d.line((right-1, y, right-1, bottom-1), fill=color(-32))
                d.line((x, y, right-2, y), fill=color(delta+12))
    elif pattern == 'soil':
        im = ground_tile('soil', base)
    elif pattern in ('grass', 'grass_side'):
        im = ground_tile('grass', base, grass=True)
        if pattern == 'grass_side':
            grass = im
            # Use the actual soil specification so side and underside stay in sync.
            import json
            from pathlib import Path
            specs = json.loads((Path(__file__).resolve().parents[1]/'data/base_textures.json').read_text())
            im = ground_tile('soil', ImageColor.getrgb(specs['soil']['color']))
            # Periodic fringe, including matching first/last columns. No bright rim.
            depths = [3, 3, 4, 5, 4, 3, 3, 4, 4, 3, 2, 3, 4, 4, 3]
            for x in range(16):
                depth = depths[x % 15]
                for y in range(depth+1):
                    pixel = grass.getpixel((x, y))
                    im.putpixel((x, y), tuple(c-8 for c in pixel[:3])+(255,) if y==depth else pixel)
    elif pattern == 'sand':
        for y in (3,8,13):
            for x in range(16):
                d.point((x,(y+int(math.sin(x*.45)*1.5))%16),fill=color(14))
    elif pattern == 'bark':
        for x in (1,5,10,14):
            bend=rng.choice([-1,1])
            d.line((x,0,x,5,x+bend,10,x+bend,15),fill=color(-34))
            d.line((x+1,0,x+1,5,x+1+bend,10,x+1+bend,15),fill=color(18))
        if name.startswith('birch'):
            for x,y,w in [(0,3,4),(9,7,5),(3,12,4),(12,14,3)]:
                d.line((x,y,x+w,y),fill=(64,57,61,255))
        else:
            d.rectangle((6,7,8,10),outline=color(-35))
    elif pattern == 'rings':
        for y in range(16):
            for x in range(16):
                radius=math.sqrt((x-7.0)**2+(y-8.0)**2)
                if radius>7.0: shade=-48
                elif int(radius*1.35)%3==0: shade=-24
                else: shade=8+rng.randrange(-5,6)
                d.point((x,y),fill=color(shade))
        d.line((8,8,11,10,14,10),fill=color(-38))
    elif pattern == 'leaves':
        im=Image.new('RGBA',(16,16),(0,0,0,0));d=ImageDraw.Draw(im)
        for row in range(5):
            for col in range(5):
                x=col*3+rng.randrange(2);y=row*3+rng.randrange(2)
                d.polygon([(x,y+1),(x+1,y),(x+3,y+1),(x+1,y+3)],fill=color(rng.randrange(-24,18)))
                d.point((x+1,y+1),fill=color(25))
    elif pattern == 'basalt_side':
        for x in (0,5,10,15):
            d.line((x,0,x,15),fill=color(-35))
            if x<15: d.line((x+1,0,x+1,15),fill=color(18))
        for x,y in [(1,5),(6,11),(11,7)]:
            d.line((x,y,x+3,y),fill=color(-25))
    elif pattern.startswith('pumpkin'):
        if pattern=='pumpkin_side':
            for x in (0,4,8,12):
                d.line((x,0,x,15),fill=color(-35))
                d.line((x+2,0,x+2,15),fill=color(18))
        else:
            for x,y in [(0,0),(15,0),(0,15),(15,15)]:
                d.line((x,y,8,8),fill=color(-28))
            if pattern=='pumpkin_top':
                d.rectangle((6,6,9,9),fill=(87,107,51,255))
                d.line((7,6,7,8),fill=(139,154,76,255))
            else:
                d.rectangle((6,6,9,9),fill=(137,92,45,255))
                d.point((7,7),fill=(193,145,74,255))
    elif pattern=='water':
        for y in range(16):
            for x in range(16):
                wave=math.sin(x*math.tau/16+y*math.tau/8)*8+math.cos(y*math.tau/8)*5
                d.point((x,y),fill=color(round(wave)))
        for x,y,w in [(1,3,3),(8,9,4),(3,14,2)]:
            d.line((x,y,x+w,y),fill=color(26))
    elif pattern=='short_grass':
        im=Image.new('RGBA',(16,16),(0,0,0,0));d=ImageDraw.Draw(im)
        for x,top,bend in [(2,8,-1),(5,4,-2),(8,6,1),(11,3,2),(14,9,1)]:
            d.line((x,15,x,top+3,x+bend,top),fill=color(rng.randrange(-15,22)))
            d.line((x+1,15,x+1,top+4),fill=color(25))
    elif pattern=='placeholder':
        d.rectangle((1,1,14,14),outline=color(35))
        d.line((3,12,7,3,12,12,3,12),fill=color(-35))
    else:
        raise ValueError(f'Unknown base pattern: {pattern}')
    return im.resize((64,64),Image.Resampling.NEAREST)
