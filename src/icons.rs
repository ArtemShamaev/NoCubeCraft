//! Item icons, painted at startup from simple shapes.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::items::Item;

const SIZE: usize = 48;

pub struct IconsPlugin;

impl Plugin for IconsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
    }
}

#[derive(Resource)]
pub struct ItemIcons(HashMap<Item, Handle<Image>>);

/// Hearts and drumsticks for health and hunger: empty, half and full.
#[derive(Resource)]
pub struct VitalIcons {
    pub heart: [Handle<Image>; 3],
    pub food: [Handle<Image>; 3],
}

impl ItemIcons {
    pub fn get(&self, item: Item) -> Handle<Image> {
        self.0[&item].clone()
    }
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut add = |canvas: Canvas| {
        images.add(Image::new(
            Extent3d { width: SIZE as u32, height: SIZE as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            canvas.pixels.into_iter().flat_map(to_srgb8).collect(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        ))
    };
    let icons = Item::ALL
        .into_iter()
        .map(|item| {
            let mut canvas = Canvas::new();
            paint(&mut canvas, item);
            (item, add(canvas))
        })
        .collect();
    commands.insert_resource(ItemIcons(icons));
    let vital = |shape: fn(Vec2) -> f32, color: Rgb| {
        [0.0, 0.5, 1.0].map(|fill| {
            let mut c = Canvas::new();
            c.shape(shape, [0.12, 0.1, 0.1]);
            if fill > 0.0 {
                c.shape(|p| shape(p).max(p.x - fill), color);
            }
            c
        })
    };
    let [h0, h1, h2] = vital(heart, [0.85, 0.12, 0.12]);
    let [f0, f1, f2] = vital(drumstick, [0.72, 0.42, 0.2]);
    commands.insert_resource(VitalIcons { heart: [add(h0), add(h1), add(h2)], food: [add(f0), add(f1), add(f2)] });
}

fn heart(p: Vec2) -> f32 {
    let q = p - Vec2::new(0.5, 0.5);
    let lobes = (q - Vec2::new(-0.17, -0.1)).length().min((q - Vec2::new(0.17, -0.1)).length()) - 0.21;
    lobes.min(rotated_square(p, Vec2::new(0.5, 0.48), 0.36))
}

fn drumstick(p: Vec2) -> f32 {
    let meat = (rotate(p - Vec2::new(0.6, 0.4), 0.785) / Vec2::new(0.3, 0.22)).length() - 1.0;
    let bone = segment(p, Vec2::new(0.2, 0.8), Vec2::new(0.45, 0.55)) - 0.06;
    meat.min(bone).min(p.distance(Vec2::new(0.18, 0.82)) - 0.09)
}

type Rgb = [f32; 3];

const STONE: Rgb = [0.55, 0.54, 0.52];
const IRON: Rgb = [0.82, 0.82, 0.86];
const WOOD: Rgb = [0.55, 0.38, 0.22];
const DARK_WOOD: Rgb = [0.36, 0.24, 0.13];

fn paint(c: &mut Canvas, item: Item) {
    match item {
        Item::Dirt => c.lump([0.45, 0.32, 0.21], 0.25),
        Item::Stone => c.lump(STONE, 0.18),
        Item::Sand => c.lump([0.86, 0.78, 0.55], 0.1),
        Item::Planks => {
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::new(0.36, 0.36), 0.04), WOOD);
            for y in [0.38, 0.62] {
                c.shape(|p| segment(p, Vec2::new(0.16, y), Vec2::new(0.84, y)) - 0.012, DARK_WOOD);
            }
        }
        Item::Bricks => {
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.36), 0.04), [0.62, 0.36, 0.3]);
            for y in [0.38, 0.5, 0.62] {
                c.shape(|p| segment(p, Vec2::new(0.16, y), Vec2::new(0.84, y)) - 0.01, [0.8, 0.78, 0.74]);
            }
            for (x, y0, y1) in [(0.4, 0.26, 0.38), (0.6, 0.38, 0.5), (0.4, 0.5, 0.62), (0.6, 0.62, 0.74)] {
                c.shape(|p| segment(p, Vec2::new(x, y0), Vec2::new(x, y1)) - 0.01, [0.8, 0.78, 0.74]);
            }
        }
        Item::Snow => c.lump([0.94, 0.96, 1.0], 0.05),
        Item::Sandstone => {
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.34), 0.04), [0.85, 0.72, 0.48]);
            for y in [0.4, 0.6] {
                c.shape(|p| segment(p, Vec2::new(0.18, y), Vec2::new(0.82, y)) - 0.01, [0.7, 0.56, 0.36]);
            }
        }
        Item::Gravel => {
            c.lump([0.55, 0.52, 0.47], 0.35);
            c.speckle([0.35, 0.33, 0.3], 0.7);
        }
        Item::Clay => c.lump([0.72, 0.5, 0.36], 0.08),
        Item::Ice => c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.32), 0.06), [0.7, 0.88, 0.95]),
        Item::Glue => {
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.58), Vec2::new(0.2, 0.24), 0.06), [0.85, 0.9, 0.8]);
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.28), Vec2::new(0.07, 0.08), 0.02), [0.9, 0.55, 0.2]);
            c.shape(|p| p.distance(Vec2::new(0.5, 0.62)) - 0.1, [0.45, 0.85, 0.3]);
        }
        Item::Brace => {
            let frame = [0.82, 0.6, 0.35];
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.34), 0.03).abs() - 0.05, frame);
            c.shape(|p| segment(p, Vec2::new(0.2, 0.2), Vec2::new(0.8, 0.8)) - 0.05, frame);
        }
        Item::Activator => {
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.36), 0.05), [0.3, 0.45, 0.5]);
            c.shape(|p| p.distance(Vec2::splat(0.5)) - 0.16, [0.3, 0.95, 1.0]);
            c.shape(|p| p.distance(Vec2::splat(0.5)) - 0.07, [0.9, 1.0, 1.0]);
        }
        Item::Shardstone => c.lump([0.55, 0.38, 0.72], 0.18),
        Item::Core => {
            c.shape(|p| rotated_square(p, Vec2::splat(0.5), 0.3), [0.35, 0.95, 1.0]);
            c.shape(|p| p.distance(Vec2::splat(0.5)) - 0.1, [0.95, 1.0, 1.0]);
        }
        Item::Rift => {
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.36), 0.05), [0.25, 0.12, 0.35]);
            c.shape(|p| ((p - Vec2::splat(0.5)) / Vec2::new(0.14, 0.3)).length() - 1.0, [0.75, 0.4, 1.0]);
            c.shape(|p| ((p - Vec2::splat(0.5)) / Vec2::new(0.05, 0.18)).length() - 1.0, [1.0, 0.9, 1.0]);
        }
        Item::Bow => {
            let center = Vec2::new(0.26, 0.74);
            let along = Vec2::new(1.0, -1.0).normalize();
            c.shape(
                |p| ((p.distance(center) - 0.5).abs() - 0.04).max(0.25 - (p - center).normalize().dot(along)),
                WOOD,
            );
            c.shape(|p| segment(p, Vec2::new(0.2, 0.28), Vec2::new(0.72, 0.8)) - 0.012, [0.9, 0.9, 0.85]);
        }
        Item::Arrow => {
            c.shape(|p| segment(p, Vec2::new(0.24, 0.76), Vec2::new(0.72, 0.28)) - 0.025, WOOD);
            c.shape(|p| rotated_square(p, Vec2::new(0.74, 0.26), 0.1), STONE);
            for (x, y) in [(0.18, 0.72), (0.28, 0.82)] {
                c.shape(|p| segment(p, Vec2::new(x, y), Vec2::new(0.3, 0.7)) - 0.035, [0.92, 0.92, 0.9]);
            }
        }
        Item::Bone => {
            let bone = [0.92, 0.9, 0.82];
            c.shape(|p| segment(p, Vec2::new(0.3, 0.7), Vec2::new(0.7, 0.3)) - 0.06, bone);
            for (x, y) in [(0.22, 0.7), (0.3, 0.78), (0.7, 0.22), (0.78, 0.3)] {
                c.shape(|p| p.distance(Vec2::new(x, y)) - 0.08, bone);
            }
        }
        Item::Thread => {
            c.shape(|p| (p.distance(Vec2::splat(0.5)) - 0.22).abs() - 0.03, [0.92, 0.92, 0.88]);
            c.shape(|p| segment(p, Vec2::new(0.62, 0.7), Vec2::new(0.84, 0.86)) - 0.02, [0.92, 0.92, 0.88]);
        }
        Item::Feather => {
            c.shape(
                |p| (rotate(p - Vec2::splat(0.5), 0.785) / Vec2::new(0.12, 0.34)).length() - 1.0,
                [0.95, 0.95, 0.95],
            );
            c.shape(|p| segment(p, Vec2::new(0.24, 0.76), Vec2::new(0.7, 0.3)) - 0.012, [0.6, 0.6, 0.6]);
        }
        Item::Coin => {
            c.shape(|p| p.distance(Vec2::splat(0.5)) - 0.3, [0.95, 0.75, 0.25]);
            c.shape(|p| (p.distance(Vec2::splat(0.5)) - 0.21).abs() - 0.02, [0.75, 0.55, 0.15]);
            c.shape(|p| segment(p, Vec2::new(0.5, 0.38), Vec2::new(0.5, 0.62)) - 0.04, [0.75, 0.55, 0.15]);
        }
        Item::Heart => {
            c.shape(
                |p| {
                    let q = p - Vec2::new(0.5, 0.48);
                    let lobes =
                        (q - Vec2::new(-0.12, -0.08)).length().min((q - Vec2::new(0.12, -0.08)).length()) - 0.15;
                    lobes.min(rotated_square(p, Vec2::new(0.5, 0.52), 0.26))
                },
                [0.75, 0.35, 1.0],
            );
            c.shape(|p| p.distance(Vec2::new(0.42, 0.4)) - 0.05, [1.0, 0.9, 1.0]);
        }
        Item::Cactus => {
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.55), Vec2::new(0.14, 0.32), 0.1), [0.25, 0.55, 0.2]);
            c.shape(|p| rounded_box(p, Vec2::new(0.3, 0.45), Vec2::new(0.06, 0.12), 0.05), [0.25, 0.55, 0.2]);
        }
        Item::Beef | Item::Porkchop | Item::Chicken | Item::CookedBeef | Item::CookedPorkchop | Item::CookedChicken => {
            let meat = match item {
                Item::Beef => [0.75, 0.2, 0.2],
                Item::Porkchop => [0.95, 0.6, 0.6],
                Item::Chicken => [0.95, 0.8, 0.7],
                _ => [0.55, 0.3, 0.15],
            };
            c.shape(|p| ((p - Vec2::new(0.5, 0.52)) / Vec2::new(0.34, 0.24)).length() - 1.0, meat);
            c.shape(|p| ((p - Vec2::new(0.6, 0.48)) / Vec2::new(0.1, 0.07)).length() - 1.0, [0.95, 0.9, 0.85]);
        }
        Item::Leaves => {
            c.lump([0.24, 0.45, 0.15], 0.3);
            c.speckle([0.16, 0.32, 0.1], 0.6);
        }
        Item::Coal => {
            c.lump([0.12, 0.12, 0.13], 0.2);
            c.speckle([0.3, 0.3, 0.32], 0.85);
        }
        Item::IronOre => {
            c.lump(STONE, 0.18);
            c.speckle([0.72, 0.45, 0.28], 0.7);
        }
        Item::IronIngot => {
            c.shape(|p| trapezoid(p, 0.62, 0.3, 0.42, 0.2), IRON);
            c.shape(|p| segment(p, Vec2::new(0.36, 0.48), Vec2::new(0.6, 0.48)) - 0.02, [0.97, 0.97, 1.0]);
        }
        Item::Wood => {
            c.shape(|p| segment(p, Vec2::new(0.28, 0.72), Vec2::new(0.62, 0.38)) - 0.17, WOOD);
            c.shape(|p| p.distance(Vec2::new(0.66, 0.34)) - 0.17, [0.78, 0.62, 0.4]);
            c.shape(|p| (p.distance(Vec2::new(0.66, 0.34)) - 0.09).abs() - 0.012, DARK_WOOD);
        }
        Item::Stick => c.shape(|p| segment(p, Vec2::new(0.22, 0.8), Vec2::new(0.78, 0.2)) - 0.045, WOOD),
        Item::WoodenPickaxe | Item::StonePickaxe | Item::IronPickaxe | Item::DiamondPickaxe => {
            handle(c);
            let color = head_color(item);
            // An arc across the top of the handle.
            let center = Vec2::new(0.4, 0.62);
            let radius = center.distance(Vec2::new(0.66, 0.36));
            let along = Vec2::new(1.0, -1.0).normalize();
            c.shape(
                |p| {
                    ((p.distance(center) - radius).abs() - 0.05).max((0.64 - (p - center).normalize().dot(along)) * 0.5)
                },
                color,
            );
        }
        Item::WoodenShovel | Item::StoneShovel | Item::IronShovel | Item::DiamondShovel => {
            handle(c);
            let color = head_color(item);
            c.shape(
                |p| rounded_box(rotate(p - Vec2::new(0.72, 0.28), -0.785), Vec2::ZERO, Vec2::new(0.11, 0.16), 0.08),
                color,
            );
        }
        Item::WoodenSword | Item::StoneSword | Item::IronSword | Item::DiamondSword => {
            let color = head_color(item);
            c.shape(|p| segment(p, Vec2::new(0.22, 0.78), Vec2::new(0.34, 0.66)) - 0.05, DARK_WOOD);
            c.shape(|p| segment(p, Vec2::new(0.24, 0.56), Vec2::new(0.44, 0.76)) - 0.04, DARK_WOOD);
            c.shape(|p| segment(p, Vec2::new(0.36, 0.64), Vec2::new(0.8, 0.2)) - 0.055, color);
        }
        Item::Diamond => {
            c.shape(|p| rotated_square(p, Vec2::splat(0.5), 0.26), [0.4, 0.9, 0.95]);
            c.shape(|p| rotated_square(p, Vec2::new(0.45, 0.45), 0.08), [0.85, 1.0, 1.0]);
        }
        Item::Charcoal => {
            c.lump([0.16, 0.12, 0.1], 0.25);
            c.speckle([0.35, 0.25, 0.18], 0.85);
        }
        Item::CraftingTable => {
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.62), Vec2::new(0.34, 0.24), 0.03), DARK_WOOD);
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.36), Vec2::new(0.38, 0.1), 0.03), WOOD);
            c.shape(|p| segment(p, Vec2::new(0.3, 0.52), Vec2::new(0.42, 0.7)) - 0.025, STONE);
            c.shape(|p| segment(p, Vec2::new(0.58, 0.52), Vec2::new(0.7, 0.72)) - 0.025, IRON);
        }
        Item::Chest => {
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.56), Vec2::new(0.36, 0.26), 0.04), WOOD);
            c.shape(|p| segment(p, Vec2::new(0.16, 0.46), Vec2::new(0.84, 0.46)) - 0.02, DARK_WOOD);
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.5), Vec2::new(0.06, 0.08), 0.02), [0.9, 0.78, 0.35]);
        }
        Item::WoodenAxe | Item::StoneAxe | Item::IronAxe | Item::DiamondAxe => {
            handle(c);
            let color = head_color(item);
            c.shape(
                |p| rounded_box(rotate(p - Vec2::new(0.72, 0.32), 0.785), Vec2::ZERO, Vec2::new(0.1, 0.17), 0.06),
                color,
            );
        }
        Item::Furnace => {
            c.shape(|p| rounded_box(p, Vec2::splat(0.5), Vec2::splat(0.36), 0.05), STONE);
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.62), Vec2::new(0.2, 0.13), 0.03), [0.08, 0.06, 0.05]);
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.7), Vec2::new(0.15, 0.04), 0.02), [1.0, 0.55, 0.15]);
        }
        Item::Torch => {
            c.shape(|p| segment(p, Vec2::new(0.5, 0.88), Vec2::new(0.5, 0.42)) - 0.05, WOOD);
            c.shape(|p| p.distance(Vec2::new(0.5, 0.3)) - 0.13, [1.0, 0.55, 0.15]);
            c.shape(|p| p.distance(Vec2::new(0.5, 0.33)) - 0.06, [1.0, 0.92, 0.5]);
        }
        Item::Bed => {
            c.shape(|p| rounded_box(p, Vec2::new(0.5, 0.58), Vec2::new(0.4, 0.14), 0.03), WOOD);
            c.shape(|p| rounded_box(p, Vec2::new(0.58, 0.46), Vec2::new(0.3, 0.08), 0.03), [0.7, 0.18, 0.15]);
            c.shape(|p| rounded_box(p, Vec2::new(0.2, 0.44), Vec2::new(0.1, 0.07), 0.04), [0.92, 0.9, 0.85]);
            for x in [0.14, 0.86] {
                c.shape(|p| segment(p, Vec2::new(x, 0.66), Vec2::new(x, 0.8)) - 0.035, DARK_WOOD);
            }
        }
        Item::Mutton | Item::CookedMutton => {
            let meat = if item == Item::Mutton { [0.82, 0.36, 0.36] } else { [0.55, 0.3, 0.15] };
            c.shape(|p| segment(p, Vec2::new(0.22, 0.78), Vec2::new(0.4, 0.6)) - 0.05, [0.95, 0.92, 0.85]);
            c.shape(|p| (rotate(p - Vec2::new(0.58, 0.42), 0.785) / Vec2::new(0.3, 0.22)).length() - 1.0, meat);
        }
        Item::Wool => {
            for (x, y, r) in [(0.36, 0.56, 0.2), (0.62, 0.58, 0.2), (0.5, 0.38, 0.22)] {
                c.shape(|p| p.distance(Vec2::new(x, y)) - r, [0.93, 0.92, 0.88]);
            }
        }
        Item::Seeds => {
            for (x, y) in [(0.35, 0.4), (0.6, 0.35), (0.45, 0.62), (0.68, 0.6), (0.3, 0.68)] {
                c.shape(|p| (p - Vec2::new(x, y)).length() - 0.065, [0.62, 0.66, 0.28]);
            }
        }
        Item::Wheat => {
            for x in [0.38, 0.5, 0.62] {
                c.shape(|p| segment(p, Vec2::new(0.5, 0.88), Vec2::new(x, 0.3)) - 0.025, [0.72, 0.62, 0.25]);
                c.shape(|p| ((p - Vec2::new(x, 0.26)) / Vec2::new(0.05, 0.12)).length() - 1.0, [0.9, 0.76, 0.32]);
            }
        }
        Item::Bread => {
            c.shape(|p| ((p - Vec2::new(0.5, 0.55)) / Vec2::new(0.36, 0.2)).length() - 1.0, [0.72, 0.46, 0.2]);
            for x in [0.38, 0.5, 0.62] {
                c.shape(|p| segment(p, Vec2::new(x - 0.04, 0.5), Vec2::new(x + 0.04, 0.42)) - 0.015, [0.9, 0.72, 0.42]);
            }
        }
    }
}

fn handle(c: &mut Canvas) {
    c.shape(|p| segment(p, Vec2::new(0.2, 0.82), Vec2::new(0.66, 0.36)) - 0.045, WOOD);
}

/// Colour of a tool's head: its material.
pub fn head_color(item: Item) -> Rgb {
    match item.tool().map_or(1, |t| t.tier) {
        0 => [0.66, 0.49, 0.3],
        1 => STONE,
        2 => IRON,
        _ => [0.4, 0.9, 0.95],
    }
}

/// A square turned 45 degrees, `r` from centre to corner.
fn rotated_square(p: Vec2, center: Vec2, r: f32) -> f32 {
    let q = (p - center).abs();
    (q.x + q.y - r) * std::f32::consts::FRAC_1_SQRT_2
}

struct Canvas {
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    fn new() -> Self {
        Self { pixels: vec![[0.0; 4]; SIZE * SIZE] }
    }

    /// Paints where the signed distance (in icon units, 0..1) is negative, with a dark outline.
    fn shape(&mut self, sdf: impl Fn(Vec2) -> f32, color: Rgb) {
        let px = 1.0 / SIZE as f32;
        for (i, pixel) in self.pixels.iter_mut().enumerate() {
            let p = Vec2::new((i % SIZE) as f32 + 0.5, (i / SIZE) as f32 + 0.5) * px;
            let d = sdf(p) / px;
            let fill = (0.5 - d).clamp(0.0, 1.0);
            let outline = (2.0 - d).clamp(0.0, 1.0) * 0.8;
            for (k, channel) in color.iter().enumerate() {
                let shaded = channel * (1.0 - 0.18 * p.y);
                let with_outline = shaded * fill + channel * 0.25 * (1.0 - fill);
                pixel[k] = pixel[k] * (1.0 - outline) + with_outline * outline;
            }
            pixel[3] = pixel[3].max(outline);
        }
    }

    /// An irregular chunk of material with grainy shading.
    fn lump(&mut self, color: Rgb, grain: f32) {
        self.shape(
            |p| {
                let q = p - Vec2::splat(0.5);
                let wobble = 0.03 * (q.y.atan2(q.x) * 5.0).sin();
                q.length() - 0.34 - wobble
            },
            color,
        );
        for (i, pixel) in self.pixels.iter_mut().enumerate() {
            let n = hash(i as u32) * 2.0 - 1.0;
            for channel in &mut pixel[..3] {
                *channel *= 1.0 + n * grain;
            }
        }
    }

    /// Dots of `color` over existing paint.
    fn speckle(&mut self, color: Rgb, threshold: f32) {
        for (i, pixel) in self.pixels.iter_mut().enumerate() {
            let cell = (i % SIZE / 5) as u32 * 31 + (i / SIZE / 5) as u32 * 977;
            if pixel[3] > 0.9 && hash(cell) > threshold && hash(i as u32 ^ 0x55) > 0.3 {
                pixel[..3].copy_from_slice(&color);
            }
        }
    }
}

fn to_srgb8(p: [f32; 4]) -> [u8; 4] {
    let c = Color::linear_rgba(p[0], p[1], p[2], p[3]).to_srgba();
    [c.red, c.green, c.blue, c.alpha].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn hash(x: u32) -> f32 {
    let mut h = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277_803_737);
    ((h >> 22) ^ h) as f32 / u32::MAX as f32
}

fn segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
    p.distance(a + (b - a) * t)
}

fn rounded_box(p: Vec2, center: Vec2, half: Vec2, radius: f32) -> f32 {
    let q = (p - center).abs() - half + radius;
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - radius
}

/// Isosceles trapezoid centred at (0.5, `y`), `bottom` and `top` widths, `height` tall.
fn trapezoid(p: Vec2, y: f32, top: f32, bottom: f32, height: f32) -> f32 {
    let t = ((p.y - (y - height)) / (2.0 * height)).clamp(0.0, 1.0);
    let half = (top + (bottom - top) * t) * 0.5;
    ((p.x - 0.5).abs() - half).max((p.y - y).abs() - height)
}

fn rotate(p: Vec2, angle: f32) -> Vec2 {
    Vec2::from_angle(angle).rotate(p)
}
