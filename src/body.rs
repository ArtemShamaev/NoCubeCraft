//! Kinematic capsules that walk over the terrain and swim: the player and creatures.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    AppState,
    terrain::{CELL, SHARDS_Y, Shape, Terrain},
};

const GROUND_ACCELERATION: f32 = 14.0;
const AIR_ACCELERATION: f32 = 2.0;
const WATER_ACCELERATION: f32 = 4.0;
const GRAVITY: f32 = 24.0;
/// Clears 1.25 m, a block and a bit.
const JUMP_SPEED: f32 = 7.75;
/// Sneaking never steps off a drop deeper than this.
const SNEAK_DROP: f32 = 0.6;
const MAX_FALL_SPEED: f32 = 50.0;
/// cos(50°): steeper surfaces are walls, not ground.
const MIN_GROUND_NORMAL_Y: f32 = 0.64;
/// How far down we look for ground to stick to while walking downhill.
const GROUND_SNAP: f32 = 0.4;
const SWIM_SPEED_FACTOR: f32 = 0.55;
const SINK_SPEED: f32 = 1.2;
const SWIM_UP_SPEED: f32 = 3.5;
/// How quickly vertical speed in water settles to sinking or swimming up.
const WATER_DRAG: f32 = 3.0;
/// Ledges up to this high are walked onto without jumping (two 25 cm blocks).
const STEP_HEIGHT: f32 = 0.55;
/// How far forward a climb moves at once.
const STEP_REACH: f32 = 0.25;
const FLY_ACCELERATION: f32 = 8.0;
const FLY_VERTICAL_SPEED: f32 = 8.0;

pub struct BodyPlugin;

impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, move_bodies.in_set(MoveBodies).run_if(in_state(AppState::Game)));
    }
}

#[derive(PhysicsLayer, Default)]
pub enum Layer {
    #[default]
    Solid,
    /// Small props bodies walk through, such as torches and crops. Rays still hit them.
    Walkthrough,
    /// Pieces of the world falling down; they only hit solid things.
    Falling,
}

/// Controllers that set [`Body::wish`] and [`Body::jump`] run before this.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MoveBodies;

/// Transform is at the feet.
#[derive(Component)]
pub struct Body {
    pub radius: f32,
    pub height: f32,
    /// Desired horizontal velocity.
    pub wish: Vec3,
    /// Held jump: jumps off the ground, swims up in water, rises when flying.
    pub jump: bool,
    /// Sinks when flying.
    pub descend: bool,
    /// Sneaking: won't walk off an edge.
    pub sneak: bool,
    /// Sprinting, for the view to show it.
    pub sprint: bool,
    /// No gravity; moves freely (creative mode).
    pub flying: bool,
    pub velocity: Vec3,
    pub ground_normal: Option<Vec3>,
    pub in_water: bool,
    /// Downward speed on landing this frame, zero otherwise.
    pub impact: f32,
    /// Fastest downward speed since leaving the ground.
    fall_speed: f32,
}

impl Body {
    pub fn new(radius: f32, height: f32) -> Self {
        Self {
            radius,
            height,
            wish: Vec3::ZERO,
            jump: false,
            descend: false,
            sneak: false,
            sprint: false,
            flying: false,
            velocity: Vec3::ZERO,
            ground_normal: None,
            in_water: false,
            impact: 0.0,
            fall_speed: 0.0,
        }
    }

    pub fn collider(&self) -> Collider {
        Collider::capsule(self.radius, self.height - 2.0 * self.radius)
    }

    pub fn center_offset(&self) -> Vec3 {
        Vec3::Y * self.height * 0.5
    }
}

fn move_bodies(
    time: Res<Time>,
    terrain: Res<Terrain>,
    move_and_slide: MoveAndSlide,
    mut bodies: Query<(Entity, &mut Body, &mut Transform)>,
) {
    let dt = time.delta_secs();
    if dt == 0.0 {
        return;
    }
    for (entity, mut body, mut transform) in &mut bodies {
        let body = &mut *body;
        // Ground that isn't collidable yet would let bodies fall through; wait for it.
        let ahead = transform.translation + body.velocity * dt - Vec3::Y;
        if !terrain.has_collision_at(transform.translation) || !terrain.has_collision_at(ahead) {
            continue;
        }
        let offset = body.center_offset();
        let shape = body.collider();
        let filter = SpatialQueryFilter::from_mask(Layer::Solid).with_excluded_entities([entity]);
        if body.flying {
            let vertical = (body.jump as i32 - body.descend as i32) as f32 * FLY_VERTICAL_SPEED;
            let target = body.wish.with_y(vertical);
            body.velocity = body.velocity.lerp(target, 1.0 - (-FLY_ACCELERATION * dt).exp());
            let out = move_and_slide.move_and_slide(
                &shape,
                transform.translation + offset,
                Quat::IDENTITY,
                body.velocity,
                time.delta(),
                &MoveAndSlideConfig::default(),
                &filter,
                |_| MoveAndSlideHitResponse::Accept,
            );
            transform.translation = out.position - offset;
            body.velocity = out.projected_velocity;
            body.ground_normal = None;
            body.fall_speed = 0.0;
            body.impact = 0.0;
            body.in_water = false;
            continue;
        }
        body.in_water = terrain.is_water(transform.translation + offset);
        body.impact = 0.0;
        let was_grounded = body.ground_normal.is_some();

        let (wish, acceleration) = match (body.in_water, was_grounded) {
            (true, _) => (body.wish * SWIM_SPEED_FACTOR, WATER_ACCELERATION),
            (false, true) => (body.wish, GROUND_ACCELERATION),
            (false, false) => (body.wish, AIR_ACCELERATION),
        };
        let horizontal = body.velocity.with_y(0.0).lerp(wish, 1.0 - (-acceleration * dt).exp());

        // The Shards pull only gently.
        let gravity = if transform.translation.y > SHARDS_Y - 500.0 { GRAVITY * 0.45 } else { GRAVITY };
        let jumped = was_grounded && body.jump && !body.in_water;
        let mut velocity = if body.in_water {
            let target = if body.jump { SWIM_UP_SPEED } else { -SINK_SPEED };
            horizontal.with_y(body.velocity.y.lerp(target, 1.0 - (-WATER_DRAG * dt).exp()))
        } else {
            match body.ground_normal {
                Some(_) if jumped => horizontal.with_y(JUMP_SPEED),
                // Follow the slope instead of pushing into it; no gravity, so standing still doesn't slide.
                Some(normal) => {
                    let along = horizontal - normal * horizontal.dot(normal);
                    along.normalize_or_zero() * horizontal.length()
                }
                None => horizontal.with_y((body.velocity.y - gravity * dt).max(-MAX_FALL_SPEED)),
            }
        };
        body.fall_speed = body.fall_speed.max(-velocity.y);

        let start = transform.translation + offset;
        let out = move_and_slide.move_and_slide(
            &shape,
            transform.translation + offset,
            Quat::IDENTITY,
            velocity,
            time.delta(),
            &MoveAndSlideConfig::default(),
            &filter,
            |_| MoveAndSlideHitResponse::Accept,
        );
        let mut center = out.position;
        velocity = out.projected_velocity;

        // Sneaking: keep whichever part of the move still has ground under it.
        if body.sneak && was_grounded && !jumped && !body.in_water {
            // Ground straight under the middle of the feet, as a capsule rolls off any edge.
            let supported = |at: Vec3| {
                let feet = at - offset + Vec3::Y * 0.05;
                move_and_slide.spatial_query.cast_ray(feet, Dir3::NEG_Y, SNEAK_DROP + 0.05, true, &filter).is_some()
            };
            if !supported(center) {
                let moved = center - start;
                let keep = [Vec3::new(moved.x, moved.y, 0.0), Vec3::new(0.0, moved.y, moved.z)]
                    .into_iter()
                    .find(|m| supported(start + *m))
                    .unwrap_or(Vec3::new(0.0, moved.y, 0.0));
                center = start + keep;
                let kept = |d: f32| if d == 0.0 { 0.0 } else { 1.0 };
                velocity *= Vec3::new(kept(keep.x), 1.0, kept(keep.z));
            }
        }

        // Blocked by a low ledge while walking: climb onto it in one decisive step, far enough
        // that the capsule ends up resting on the ledge's top rather than its edge.
        let wanted = horizontal * dt;
        let moved = (center - start).with_y(0.0);
        if was_grounded && !jumped && !body.in_water && wanted.length() > 1e-3 && moved.length() < wanted.length() * 0.5
        {
            let config = |distance: f32| ShapeCastConfig {
                ignore_origin_penetration: true,
                ..ShapeCastConfig::from_max_distance(distance)
            };
            let cast = |from: Vec3, dir: Dir3, distance: f32| {
                move_and_slide.spatial_query.cast_shape(&shape, from, Quat::IDENTITY, dir, &config(distance), &filter)
            };
            let up = cast(start, Dir3::Y, STEP_HEIGHT).map_or(STEP_HEIGHT, |h| h.distance);
            let raised = start + Vec3::Y * (up - 0.01).max(0.0);
            let dir = Dir3::new(wanted.with_y(0.0)).unwrap_or(Dir3::NEG_Z);
            let step = wanted.length().max(STEP_REACH);
            let ahead = cast(raised, dir, step).map_or(step, |h| h.distance);
            if ahead >= STEP_REACH * 0.8 {
                let forward = raised + dir * (ahead - 0.01);
                let down = cast(forward, Dir3::NEG_Y, up + 0.05);
                if let Some(hit) = down.filter(|h| h.normal1.y >= MIN_GROUND_NORMAL_Y && h.distance < up - 0.02) {
                    center = forward - Vec3::Y * (hit.distance - 0.01);
                    velocity = horizontal;
                }
            }
        }

        // Snap down only if we were already on the ground (walking uphill moves us up too),
        // so stepping off a cliff still falls.
        let probe = if was_grounded && !body.in_water { GROUND_SNAP } else { 0.05 };
        body.ground_normal = None;
        if (was_grounded && !jumped) || velocity.y <= 0.0 {
            let hit = move_and_slide.spatial_query.cast_shape(
                &shape,
                center,
                Quat::IDENTITY,
                Dir3::NEG_Y,
                &ShapeCastConfig::from_max_distance(probe),
                &filter,
            );
            if let Some(hit) = hit.filter(|h| h.normal1.y >= MIN_GROUND_NORMAL_Y) {
                center.y -= (hit.distance - 0.01).max(0.0);
                body.ground_normal = Some(hit.normal1);
                velocity.y = velocity.y.max(0.0);
                if !was_grounded && !body.in_water {
                    body.impact = body.fall_speed.max(0.0);
                }
            }
        }
        if body.ground_normal.is_some() || body.in_water {
            body.fall_speed = 0.0;
        }

        transform.translation = center - offset;
        body.velocity = velocity;
        // A block placed into a body would let it slip out through the ground's underside.
        if let Some(feet) = free_above(&terrain, transform.translation, body.height) {
            transform.translation.y = feet;
            body.velocity.y = 0.0;
        }
    }
}

/// If a body standing at `feet` is stuck in a block, the height of the nearest place
/// above where it fits.
fn free_above(terrain: &Terrain, feet: Vec3, height: f32) -> Option<f32> {
    let solid = |cell: IVec3| {
        let block = terrain.block(cell);
        !block.is_air() && block.shape == Shape::CUBE
    };
    let stuck = ((feet + Vec3::Y * (height / 2.0).min(0.3)) / CELL).floor().as_ivec3();
    if !solid(stuck) {
        return None;
    }
    let tall = (height / CELL).ceil() as i32;
    (1..64)
        .map(|up| stuck + IVec3::Y * up)
        .find(|base| (0..tall).all(|i| !solid(*base + IVec3::Y * i)))
        .map(|base| base.y as f32 * CELL + 0.01)
}
