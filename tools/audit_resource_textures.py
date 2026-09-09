"""Verify resource provenance, resolved faces, and the packed runtime atlas."""
import csv
import hashlib
import json
from pathlib import Path
from PIL import Image
from build_resources import ROOT, EXTRA_TEXTURES, expected_images, PROVENANCE_INPUTS


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_sources():
    images = expected_images()
    manifest = json.loads((ROOT/'textures/texture_provenance.json').read_text())
    for source in PROVENANCE_INPUTS:
        assert manifest['generators'][source] == sha(ROOT/source), f'Regenerate textures after changing {source}'
    expected_paths = {f'textures/{name}.png' for name in images}
    actual_paths = {p.relative_to(ROOT).as_posix() for p in (ROOT/'textures').glob('*.png')}
    assert actual_paths == expected_paths, f'Unmanaged/missing PNGs: {actual_paths ^ expected_paths}'
    assert set(manifest['files']) == expected_paths
    for name, expected in images.items():
        path = ROOT/'textures'/f'{name}.png'
        assert sha(path) == manifest['files'][f'textures/{name}.png']['sha256'], f'Modified texture: {name}'
        with Image.open(path) as actual:
            assert actual.mode == 'RGBA' and actual.size == (64,64), name
            assert actual.tobytes() == expected.tobytes(), f'Non-reproducible texture: {name}'
        if any(token in name for token in ('leaves','short_grass')):
            alpha=expected.getchannel('A')
            assert alpha.getextrema()==(0,255), f'Missing cutout transparency: {name}'
    with (ROOT/'textures/blocks.csv').open(newline='') as f:
        rows=list(csv.DictReader(f))
    names=set(EXTRA_TEXTURES)
    for row in rows:
        faces=[row[col].strip() or row['texture'].strip() for col in ('top','side','bottom')]
        assert all(face in images for face in faces), f'Unverified face: {row["id"]}'
        names.update(faces)
        names.add(row['texture'].strip())
        if row['id'].endswith('_wood'):
            species=row['id'].removesuffix('_wood')
            assert faces==[species+'_log_top',species+'_log',species+'_log_top']
            assert images[faces[0]].tobytes()!=images[faces[1]].tobytes()
        if row['id']=='grass': assert faces==['grass','grass_block_side','soil']
        if row['id']=='pumpkin': assert faces==['pumpkin_top','pumpkin_side','pumpkin_bottom']
        if row['id']=='basalt': assert faces==['basalt_top','basalt_side','basalt_top']
    return images, sorted(names)


def verify_atlas():
    images, names = verify_sources()
    from build_atlas import COLS, TILE
    rows=(len(names)+1+COLS-1)//COLS
    expected=Image.new('RGBA',(COLS*TILE,rows*TILE))
    for i,name in enumerate(names):
        expected.paste(images[name],((i%COLS)*TILE,(i//COLS)*TILE))
    i=len(names)
    expected.paste(Image.new('RGBA',(TILE,TILE),(255,255,255,255)),((i%COLS)*TILE,(i//COLS)*TILE))
    with Image.open(ROOT/'assets/textures/atlas.png') as actual:
        assert actual.size==expected.size and actual.convert('RGBA').tobytes()==expected.tobytes(), 'Atlas does not match verified sources'
    backup=ROOT/'assets/textures'/'atlas \u2014 kopia.png'
    if backup.exists(): assert sha(backup)==sha(ROOT/'assets/textures/atlas.png'), 'Legacy atlas copy remains'
    print(f'PASS: {len(images)} reproducible source PNGs, {len(names)} active face tiles + white; all atlas pixels verified.')


if __name__=='__main__':
    verify_atlas()
