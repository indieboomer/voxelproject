"""Build resource recipes, metadata, deposits and original pixel tiles from data/resources.json.

Existing texture art is preserved. New tiles are deterministic 16px pixel designs
upscaled with nearest-neighbor sampling to the game's 64px tile size.
Run build_atlas.py and gen_world_api.py after this script.
"""
import csv
import hashlib
import json
import random
from pathlib import Path
from PIL import Image, ImageDraw, ImageColor

ROOT = Path(__file__).resolve().parent.parent
DATA = json.loads((ROOT / 'data/resources.json').read_text())
ELEMENTS = ['Earth', 'Fire', 'Water', 'Life', 'Death']


def variant(identifier):
    return ''.join(w.title() for w in identifier.split('_')) if identifier != 'redstone' else 'RedStone'


def tile(identifier, spec):
    rng = random.Random(int.from_bytes(hashlib.sha256(identifier.encode()).digest()[:8], 'little'))
    base = ImageColor.getrgb(spec['color'])
    pattern = spec['pattern']
    def shade(delta):
        return tuple(max(0, min(255, c + delta)) for c in base) + (255,)
    im = Image.new('RGBA', (16, 16), shade(0))
    draw = ImageDraw.Draw(im)
    for y in range(16):
        for x in range(16):
            draw.point((x, y), fill=shade(rng.choice([-15, -8, 0, 0, 7, 12])))
    if pattern == 'ore':
        for y in range(16):
            for x in range(16):
                v = rng.randrange(62, 86)
                draw.point((x, y), fill=(v, v + 1, v + 3, 255))
        for _ in range(9):
            x, y = rng.randrange(1, 14), rng.randrange(1, 14)
            draw.rectangle((x, y, x + 2, y + 1), fill=shade(-28))
            draw.line((x, y, x + 1, y), fill=shade(28))
            draw.point((x + 1, y + 1), fill=shade(0))
    elif pattern in ('crystal', 'glass'):
        for x, y in [(2, 3), (9, 1), (7, 9), (13, 12)]:
            draw.polygon([(x,y),(x+2,y-2),(x+3,y+2),(x+1,y+4)], fill=shade(-25))
            draw.line((x,y,x+2,y-2,x+2,y+2), fill=shade(45), width=1)
        if pattern == 'glass':
            draw.rectangle((0, 0, 15, 15), outline=shade(-38))
            draw.line((2, 6, 6, 2), fill=shade(50))
    elif pattern == 'metal':
        for y in (0, 8):
            draw.rectangle((0, y, 15, y+7), outline=shade(-42))
            draw.line((1, y+1, 14, y+1), fill=shade(32))
            draw.line((1, y+2, 1, y+5), fill=shade(16))
            draw.line((2, y+6, 14, y+6), fill=shade(-20))
    elif pattern == 'planks':
        for y in (0, 5, 10, 15):
            draw.line((0,y,15,y), fill=shade(-48))
        for x,y in [(4,1),(11,6),(7,11)]:
            draw.line((x,y,x,y+3),fill=shade(-33))
            draw.line((x+2,y+1,15,y+1),fill=shade(18))
    elif pattern in ('fiber', 'cloth'):
        for y in range(0, 16, 2):
            for x in range(0, 16, 2):
                draw.line((x,y,x+1,y),fill=shade(20 if (x+y)%4 else -28))
                draw.point((x,y+1),fill=shade(-12))
    elif pattern == 'plant':
        im = Image.new('RGBA',(16,16),(0,0,0,0)); draw = ImageDraw.Draw(im)
        for x, top in [(3,5),(7,2),(11,4)]:
            draw.line((x,15,x,top), fill=(85,119,61,255))
            draw.line((x,11,x-2,9),fill=shade(-15))
            draw.line((x,8,x+2,6),fill=shade(15))
            draw.rectangle((x-1,top,x+1,top+2),fill=shade(22))
    elif pattern in ('fern','flower','clover','cattail','mushroom','bush','shrub'):
        im=Image.new('RGBA',(16,16),(0,0,0,0)); draw=ImageDraw.Draw(im)
        green=(72,116,57,255)
        if pattern=='fern':
            draw.line((8,15,8,2),fill=shade(15))
            for y in (5,8,11):
                span=2+y//3
                draw.line((8,y+2,8-span,y-1),fill=shade(-10))
                draw.line((8,y+2,8+span,y-1),fill=shade(15))
                draw.line((8-span,y-1,8-span+1,y+1),fill=shade(5))
                draw.line((8+span,y-1,8+span-1,y+1),fill=shade(30))
        elif pattern=='mushroom':
            for x,y,size in [(5,7,4),(11,11,3)]:
                draw.rectangle((x-1,y,x+1,15),fill=(188,184,147,255))
                draw.polygon([(x-size,y),(x-size+1,y-3),(x,y-4),(x+size-1,y-3),(x+size,y)],fill=shade(-15))
                draw.line((x-size,y,x+size,y),fill=shade(30))
                draw.point((x-1,y-2),fill=shade(60)); draw.point((x+2,y-1),fill=shade(45))
        elif pattern=='cattail':
            for x,y in [(4,3),(10,1)]:
                draw.line((x,15,x,y),fill=green)
                draw.rectangle((x-1,y,x+1,y+5),fill=shade(-15))
                draw.line((x-1,y,x-1,y+4),fill=shade(25))
            draw.line((7,15,6,7),fill=(117,146,69,255));draw.line((12,15,14,8),fill=green)
        elif pattern=='clover':
            for x,y in [(3,10),(8,7),(12,11)]:
                draw.line((x,15,x,y),fill=green)
                for dx,dy in [(-2,0),(1,0),(0,-2)]:
                    draw.rectangle((x+dx,y+dy,x+dx+1,y+dy+1),fill=shade(15))
        elif pattern=='flower':
            for x,y in [(4,6),(10,3),(12,10)]:
                draw.line((x,15,x,y+1),fill=green)
                draw.line((x,12,x-2,10),fill=(105,145,71,255))
                if identifier=='lavender':
                    draw.rectangle((x-1,y-2,x+1,y+2),fill=shade(-15));draw.line((x-1,y-2,x-1,y+1),fill=shade(35))
                elif identifier=='bluebell':
                    draw.rectangle((x-1,y,x+2,y+2),fill=shade(0));draw.point((x,y-1),fill=shade(30))
                else:
                    draw.rectangle((x-2,y-1,x+2,y+1),fill=shade(5));draw.rectangle((x-1,y-2,x+1,y+2),fill=shade(20));draw.point((x,y),fill=(69,47,42,255))
        else:
            draw.line((8,15,7,4),fill=(120,88,54,255),width=2)
            for x,y in [(2,7),(12,6),(3,11),(14,10),(9,2)]:
                draw.line((8,13,x,y),fill=shade(-25))
                if pattern=='bush':
                    draw.rectangle((x-1,y-1,x+1,y+1),fill=shade(10));draw.point((x+1,y-2),fill=(210,199,141,255))
                else: draw.line((x,y,x-1,y-2),fill=shade(20))
    elif pattern == 'veined':
        for offset in (0, 7, 13):
            points=[((y//3+offset)%16,y) for y in range(16)]
            draw.line(points,fill=shade(28))
    elif pattern == 'ceramic':
        draw.rectangle((0,0,15,15),outline=shade(-32))
        draw.rectangle((2,2,13,13),outline=shade(18))
    elif pattern == 'rune':
        draw.line((5,12,5,3,10,6,5,8,10,12),fill=(138,220,218,255),width=1)
        draw.point((10,3),fill=(204,239,215,255))
    im = im.resize((64,64), Image.Resampling.NEAREST)
    im.save(ROOT / 'textures' / f'{identifier}.png')
    return im


def main():
    ids = [r['id'] for r in DATA]
    assert len(ids) == len(set(ids)) == 82
    crafting_path = ROOT / 'data/crafting.json'
    crafting = json.loads(crafting_path.read_text())
    # Preserve creature rules; the catalog owns all stackable resource definitions.
    recipes = [r for r in crafting['recipes'] if r['output']['kind'] == 'creature']
    compositions = [c for c in crafting['compositions'] if c['kind'] == 'creature' or c['id'] not in ids]
    formulas = {tuple((s['element'],s['amount']) for s in r['inputs']) for r in recipes}
    csv_path = ROOT / 'textures/blocks.csv'
    with csv_path.open(newline='') as f:
        reader = csv.DictReader(f); fields = reader.fieldnames; rows = list(reader)
    known = {r['id'].replace(' ','_'):r for r in rows}
    metadata = ['// Generated by tools/build_resources.py.', 'use super::block::BlockType;',
                'pub struct ResourceInfo { pub block: BlockType, pub category: &\'static str, pub source: &\'static str, pub location: &\'static str }',
                'pub const RESOURCES: &[ResourceInfo] = &[']
    veins = ['// Generated by tools/build_resources.py; stable salts preserve deterministic deposits.', '&[']
    surface = ['// Generated by tools/build_resources.py.', '&[']
    textures = []
    for index, r in enumerate(DATA):
        if r['id'] in known: known[r['id']]['display name']=r['name']
        slots = r['formula']
        if slots:
            assert 2 <= len(slots) <= 3, r['id']
            key = tuple((s['element'],s['amount']) for s in slots)
            assert key not in formulas, r['id']; formulas.add(key)
            cost = [sum(s['amount'] for s in slots if s['element']==e) for e in ELEMENTS]
            assert all(0 <= a <= b for a,b in zip(r['composition'],cost)), r['id']
            recipes.append(dict(id=r['id'],inputs=slots,output=dict(kind='resource',id=r['id'],quantity=1)))
        assert (r['source'] == 'natural') == (slots is None), r['id']
        compositions.append(dict(kind='resource',id=r['id'],elements=r['composition']))
        metadata.append('    ResourceInfo { block: BlockType::%s, category: %s, source: %s, location: %s },' %
                        (variant(r['id']),json.dumps(r['category']),json.dumps(r['source']),json.dumps(r['location'])))
        if 'surface' in r:
            v=r['surface']
            surface.append('    SurfaceResource { block: BlockType::%s, habitat: %s, chance: %s, salt: %s },' %
                           (variant(r['id']),json.dumps(v['habitat']),v['chance'],hex(0x73000000+index*197)))
        if 'vein' in r:
            v=r['vein']
            veins.append('    VeinConfig { block: BlockType::%s, salt: %s, attempts_per_chunk: %s, spawn_chance: %s, min_y: %s, max_y: %s, min_size: %s, max_size: %s },' %
                         (variant(r['id']),hex(0x72000000+index*193),v['attempts'],v['chance'],v['min_y'],v['max_y'],v['min_size'],v['max_size']))
        if r['texture']:
            textures.append((r['id'],tile(r['id'],r['texture'])))
            row=known.get(r['id'])
            if row is None:
                row={field:'' for field in fields}; rows.append(row)
            plant=r['texture']['pattern'] in ('plant','fern','flower','clover','cattail','mushroom','bush','shrub')
            row.update({'id':r['id'],'display name':r['name'],'resource type':r['category'].lower(),
                        'opacity':'1','emission':str(r.get('emission',0.12 if r['id'] in ['moonstone','runestone','enchanted_glass'] else 0)),
                        'roughness':'0.35' if r['texture']['pattern'] in ['metal','glass','crystal'] else '0.9',
                        'hardness':str(r['hardness']),'texture':r['id'],'only on top':'1' if plant else '',
                        'single items':'1' if plant else ''})
    for name,spec in {'mud':{'pattern':'earth','color':'#625445'},
                      'redstone':{'pattern':'rune','color':'#903c46'},
                      'crystal':{'pattern':'crystal','color':'#97d4e6'}}.items():
        textures.append((name,tile(name,spec)))
    metadata += ['];', 'pub fn info(block: BlockType) -> &\'static ResourceInfo { RESOURCES.iter().find(|r| r.block == block).expect("collectible resource metadata") }']
    veins += [']']
    surface += [']']
    (ROOT/'src/voxel/resource_surface.rs').write_text('\n'.join(surface)+'\n')
    (ROOT/'src/voxel/resource_catalog.rs').write_text('\n'.join(metadata)+'\n')
    (ROOT/'src/voxel/resource_veins.rs').write_text('\n'.join(veins)+'\n')
    crafting['recipes']=recipes; crafting['compositions']=compositions
    crafting_path.write_text(json.dumps(crafting,indent=2)+'\n')
    with csv_path.open('w',newline='') as f:
        writer=csv.DictWriter(f,fieldnames=fields); writer.writeheader(); writer.writerows(rows)
    # Contact sheet is a build artifact for visual review, with the asset names attached.
    sheet=Image.new('RGB',(8*132,((len(textures)+7)//8)*92),(27,29,34)); d=ImageDraw.Draw(sheet)
    for i,(name,im) in enumerate(textures):
        x,y=(i%8)*132,(i//8)*92; sheet.paste(im,(x+34,y),im); d.text((x+2,y+68),name,fill=(225,220,205))
    (ROOT/'target').mkdir(exist_ok=True)
    sheet.save(ROOT/'target/resource-textures.png')
    print(f'Built {len(DATA)} resources, {len(recipes)} total formulas, {len(textures)} original pixel tiles.')


if __name__ == '__main__':
    main()
