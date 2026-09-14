//! Seeded, shared recipe books. Reading records knowledge per player; books remain for friends.
use crate::{
    adventure::{cell, clear_feet, feet},
    crafting::Account,
    voxel::World,
};
use glam::Vec3;
pub const SPACING: i32 = 64;
pub const FOCUS_HEIGHT: f32 = 0.6;

fn mix(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846ca68b);
    value ^ (value >> 16)
}
#[derive(Debug, Clone, Copy)]
pub struct Book {
    pub sector: (i32, i32, u8),
    pub kind: u8,
    pub pos: Vec3,
}
/// Candidate selection uses only saved generation settings, so terrain streaming cannot move books.
pub fn locate(world: &World, sector: (i32, i32, u8)) -> Option<Book> {
    if sector.0.unsigned_abs() > 15_620 || sector.1.unsigned_abs() > 15_620 || sector.2 >= 4 {
        return None;
    }
    let kind = sector.2;
    let seed = mix(world.seed
        ^ (sector.0 as u32).wrapping_mul(73856093)
        ^ (sector.1 as u32).wrapping_mul(19349663));
    // Shuffle volumes between quadrants, then independently jitter their locations.
    // An eight-block inset keeps discoveries at least 17 blocks apart, even across sectors.
    let quadrant = (kind as u32 + seed % 4) % 4;
    for attempt in 0..16i32 {
        let salt = mix(seed
            ^ (kind as u32).wrapping_mul(0x9e3779b9)
            ^ (attempt as u32).wrapping_mul(83492791));
        let x = sector.0 * SPACING + (quadrant % 2) as i32 * 32 + 8 + (salt % 16) as i32;
        let z = sector.1 * SPACING + (quadrant / 2) as i32 * 32 + 8 + (mix(salt) % 16) as i32;
        let y = world.terrain_height(x, z) + 1;
        let water = if world.generation.shape == crate::worldgen::Shape::Mainland {
            crate::voxel::terrain::tributary(x, z, world.seed)
                .map_or(crate::voxel::world::SEA_LEVEL, |p| p.1)
        } else {
            crate::voxel::world::SEA_LEVEL
        };
        if y > water {
            return Some(Book {
                sector,
                kind,
                pos: feet((x, y, z)),
            });
        }
    }
    None
}
pub fn present(world: &World, book: Book) -> bool {
    clear_feet(world, cell(book.pos))
}
pub fn nearby(world: &World, pos: Vec3) -> Vec<Book> {
    let sx = (pos.x.floor() as i32).div_euclid(SPACING);
    let sz = (pos.z.floor() as i32).div_euclid(SPACING);
    let mut result = Vec::new();
    for x in sx - 1..=sx + 1 {
        for z in sz - 1..=sz + 1 {
            for kind in 0..4 {
                if let Some(book) = locate(world, (x, z, kind))
                    .filter(|b| b.pos.distance_squared(pos) < 96. * 96. && present(world, *b))
                {
                    result.push(book);
                }
            }
        }
    }
    result
}
pub fn can_read(world: &World, book: Book, pos: Vec3) -> bool {
    if !pos.is_finite() || !present(world, book) {
        return false;
    }
    let eye = pos + Vec3::Y * 1.62;
    let delta = book.pos + Vec3::Y * FOCUS_HEIGHT - eye;
    delta.length() <= 5.0
        && crate::raycast::raycast(world, eye, delta, (delta.length() - 0.25).max(0.)).is_none()
}
pub fn read(
    world: &World,
    account: &mut Account,
    pos: Vec3,
    sector: (i32, i32, u8),
) -> Result<u8, String> {
    let book = locate(world, sector)
        .filter(|b| can_read(world, *b, pos))
        .ok_or("Move within five blocks of the book with a clear view")?;
    let mask = 1 << book.kind;
    if account.adventure.recipe_books & mask == 0 {
        let revision = account
            .revision
            .checked_add(1)
            .ok_or("Inventory revision limit reached")?;
        account.adventure.recipe_books |= mask;
        account.revision = revision;
    }
    Ok(book.kind)
}
pub fn mesh(books: &[Book], time: f32) -> crate::voxel::mesher::MeshData {
    let mut mesh = crate::voxel::mesher::MeshData {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let source = crate::model::lore_book_mesh();
    for b in books {
        let offset = b.pos + Vec3::Y * (0.16 + 0.035 * (time * 2.).sin());
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend(source.vertices.iter().map(|v| {
            let mut v = *v;
            v.position = (Vec3::from_array(v.position) + offset).to_array();
            v
        }));
        mesh.indices.extend(source.indices.iter().map(|i| base + i));
        // Four tiny, staggered golden motes; cosmetic only and independent of world state.
        let start = mesh.vertices.len();
        for i in 0..4 {
            let phase =
                (time * 0.35 + i as f32 / 4.0 + b.pos.x * 0.13 + b.pos.z * 0.19).rem_euclid(1.0);
            let angle = i as f32 * 2.4 + time * 0.45;
            let center =
                offset + Vec3::new(angle.cos() * 0.48, 0.1 + phase * 1.05, angle.sin() * 0.32);
            let size = 0.022 * (std::f32::consts::PI * phase).sin();
            crate::voxel::mesher::push_cuboid(
                &mut mesh.vertices,
                &mut mesh.indices,
                center - Vec3::splat(size),
                center + Vec3::splat(size),
                [1.0, 0.76, 0.25],
                crate::voxel::atlas::white_uv(),
            );
        }
        for v in &mut mesh.vertices[start..] {
            v.emission = 0.8;
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::BlockType;
    fn fixture() -> (World, Book, Vec3) {
        let mut world = World::new(42);
        let book = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| (x, z, 0)))
            .find_map(|sector| locate(&world, sector))
            .unwrap();
        let p = cell(book.pos);
        crate::crafting::load_interaction_area(&mut world, book.pos);
        for x in p.0 - 3..=p.0 + 3 {
            for z in p.2 - 3..=p.2 + 3 {
                world.set_block(x, p.1 - 1, z, BlockType::Stone);
                for y in p.1..=p.1 + 3 {
                    world.set_block(x, y, z, BlockType::Air);
                }
            }
        }
        (world, book, book.pos + Vec3::X * 2.)
    }
    #[test]
    fn books_are_repeatable_shared_and_saved_per_player() {
        let (world, book, pos) = fixture();
        let mut a = Account::default();
        let mut b = Account::default();
        assert_eq!(read(&world, &mut a, pos, book.sector), Ok(book.kind));
        let once = a.clone();
        assert_eq!(read(&world, &mut a, pos, book.sector), Ok(book.kind));
        assert_eq!(a, once);
        assert_eq!(b.adventure.recipe_books, 0);
        assert_eq!(read(&world, &mut b, pos, book.sector), Ok(book.kind));
        assert_eq!(a, b);
        let mut save = crate::save::CraftingSave::default();
        save.guests.insert("reader".into(), a.clone());
        let restored: crate::save::CraftingSave =
            serde_json::from_slice(&serde_json::to_vec(&save).unwrap()).unwrap();
        assert_eq!(restored.guests["reader"], a);
        let packet = crate::net::ReliableMsg::CraftState {
            account: a.clone(),
            feedback: None,
        };
        let bytes = bincode::serialize(&packet).unwrap();
        let crate::net::ReliableMsg::CraftState { account, .. } =
            bincode::deserialize(&bytes).unwrap()
        else {
            panic!()
        };
        assert_eq!(account, a);
    }
    #[test]
    fn book_authority_rejects_remote_obstructed_and_flooded_reads() {
        let (mut world, book, pos) = fixture();
        let mut a = Account::default();
        let before = a.clone();
        assert!(read(&world, &mut a, pos + Vec3::X * 20., book.sector).is_err());
        assert!(read(&world, &mut a, Vec3::NAN, book.sector).is_err());
        assert!(read(&world, &mut a, pos, (i32::MAX, i32::MAX, 0)).is_err());
        let p = cell(book.pos);
        for y in p.1..=p.1 + 2 {
            world.set_block(p.0 + 1, y, p.2, BlockType::Stone);
        }
        assert!(read(&world, &mut a, pos, book.sector).is_err());
        assert_eq!(a, before);
        for y in p.1..=p.1 + 2 {
            world.set_block(p.0 + 1, y, p.2, BlockType::Air);
        }
        world.set_block(p.0, p.1, p.2, BlockType::Water);
        assert!(read(&world, &mut a, pos, book.sector).is_err());
        assert_eq!(a, before);
    }
    #[test]
    fn all_books_occur_on_dry_land_and_stay_put_across_streaming() {
        for shape in [
            crate::worldgen::Shape::Mainland,
            crate::worldgen::Shape::Flat,
            crate::worldgen::Shape::Islands,
            crate::worldgen::Shape::Mountains,
        ] {
            let mut w = World::new(42);
            w.generation.shape = shape;
            let mut found = 0u8;
            for x in -2..=2 {
                for z in -2..=2 {
                    for kind in 0..4 {
                        if let Some(book) = locate(&w, (x, z, kind)) {
                            crate::crafting::load_interaction_area(&mut w, book.pos);
                            if present(&w, book) {
                                found |= 1 << book.kind;
                            }
                            let before = book.pos;
                            w.chunks.clear();
                            assert_eq!(locate(&w, (x, z, kind)).unwrap().pos, before);
                        }
                    }
                }
            }
            if shape == crate::worldgen::Shape::Islands {
                // A small island may have room for only a subset; do not crowd volumes together.
                assert_ne!(found, 0, "dry islands should still have discoveries");
            } else {
                assert_eq!(found, 15, "all paths should be discoverable: {shape:?}");
            }
        }
    }
    #[test]
    fn scattered_books_have_separation_and_seeded_variety() {
        let positions = |seed| {
            let mut w = World::new(seed);
            w.generation.shape = crate::worldgen::Shape::Flat;
            (-2..=2)
                .flat_map(|x| (-2..=2).flat_map(move |z| (0..4).map(move |kind| (x, z, kind))))
                .filter_map(|sector| locate(&w, sector))
                .map(|b| glam::Vec2::new(b.pos.x, b.pos.z))
                .collect::<Vec<_>>()
        };
        let first = positions(42);
        assert_eq!(first.len(), 100);
        for (i, a) in first.iter().enumerate() {
            for b in &first[i + 1..] {
                assert!(a.distance(*b) >= 17.);
            }
        }
        assert_eq!(first, positions(42));
        assert_ne!(first, positions(43));
    }
    #[test]
    fn supplied_book_model_and_sound_are_usable() {
        use rodio::Source;
        let mesh = crate::model::lore_book_mesh();
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.indices.is_empty());
        assert!(mesh
            .indices
            .iter()
            .all(|i| (*i as usize) < mesh.vertices.len()));
        assert!(mesh
            .vertices
            .iter()
            .all(|v| v.tex_layer == 35. && Vec3::from_array(v.position).is_finite()));
        assert!(mesh.vertices.windows(2).any(|v| v[0].uv != v[1].uv));
        let min = mesh
            .vertices
            .iter()
            .fold(Vec3::splat(f32::INFINITY), |a, v| {
                a.min(Vec3::from_array(v.position))
            });
        let max = mesh
            .vertices
            .iter()
            .fold(Vec3::splat(f32::NEG_INFINITY), |a, v| {
                a.max(Vec3::from_array(v.position))
            });
        let extent = max - min;
        assert!(
            extent.y > extent.z * 2. && extent.y > 0.5,
            "book should stand upright: {extent:?}"
        );
        let book = Book {
            sector: (0, 0, 0),
            kind: 0,
            pos: Vec3::ZERO,
        };
        let first = super::mesh(&[book], 0.);
        let next = super::mesh(&[book], 1.);
        assert_eq!(first.vertices.len(), next.vertices.len());
        assert!(first.vertices.len() > mesh.vertices.len());
        assert!(first.vertices[mesh.vertices.len()..]
            .iter()
            .all(|v| v.emission > 0. && Vec3::from_array(v.position).is_finite()));
        assert_ne!(
            first.vertices.last().unwrap().position,
            next.vertices.last().unwrap().position
        );
        let decoder = rodio::Decoder::new(std::io::Cursor::new(
            include_bytes!("../sounds/lore_book.mp3").as_slice(),
        ))
        .unwrap();
        assert!(decoder.convert_samples::<f32>().any(|v| v.abs() > 0.001));
    }
}
