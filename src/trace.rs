use super::*;
use color::*;

pub type Interval = Vec2<f32>;

const TRANSMITTANCE_CUTOFF: f32 = 0.001;

fn intersect_intervals(a: Expr<Interval>, b: Expr<Interval>) -> Expr<Interval> {
    Vec2::expr(luisa::max(a.x, b.x), luisa::min(a.y, b.y))
}

#[tracked]
fn aabb_intersect(
    start: Expr<Vec2<f32>>,
    inv_dir: Expr<Vec2<f32>>,
    aabb_min: Expr<Vec2<f32>>,
    aabb_max: Expr<Vec2<f32>>,
) -> Expr<Interval> {
    let t0 = (aabb_min - start) * inv_dir;
    let t1 = (aabb_max - start) * inv_dir;
    let tmin = luisa::min(t0, t1).reduce_max();
    let tmax = luisa::max(t0, t1).reduce_min();
    Vec2::expr(tmin, tmax)
}

#[tracked]
pub fn trace_radiance(
    world: &World,
    ray_start: Expr<Vec2<f32>>,
    ray_dir: Expr<Vec2<f32>>,
    interval: Expr<Interval>,
) -> Expr<Fluence> {
    let inv_dir = (ray_dir + f32::EPSILON).recip();

    let interval = intersect_intervals(
        interval,
        aabb_intersect(
            ray_start,
            inv_dir,
            Vec2::splat_expr(0.01),
            Vec2::expr(world.width() as f32, world.height() as f32) - Vec2::splat_expr(0.01),
        ),
    );

    let ray_start = ray_start + interval.x * ray_dir;

    let pos = ray_start.floor().cast_i32();
    let pos = pos.var();

    let delta_dist = inv_dir.abs();
    let ray_step = ray_dir.signum().cast_i32();
    let side_dist =
        (ray_dir.signum() * (pos.cast_f32() - ray_start) + ray_dir.signum() * 0.5 + 0.5)
            * delta_dist;
    let side_dist = side_dist.var();

    let interval_size = interval.y - interval.x;

    let last_t = 0.0_f32.var();

    let fluence = Fluence::transparent().var();

    for _i in 0_u32.expr()..1000_u32.expr() {
        let next_t = side_dist.reduce_min();

        if world.diff.read(pos.cast_u32()) || next_t >= interval_size {
            let segment_size = luisa::min(next_t, interval_size) - last_t;
            let radiance = world.radiance.read(pos.cast_u32());
            let opacity = world.opacity.read(pos.cast_u32());
            *fluence = fluence.over(
                Color::from_comps_expr(ColorComps { radiance, opacity }).as_fluence(segment_size),
            );

            *last_t = next_t;

            if (fluence.transmittance < TRANSMITTANCE_CUTOFF).all() {
                *fluence.transmittance = Vec3::splat(0.0);
                break;
            }

            if next_t >= interval_size {
                break;
            }
        }

        let mask = side_dist <= side_dist.yx();

        *side_dist += mask.select(delta_dist, Vec2::splat_expr(0.0));
        *pos += mask.select(ray_step, Vec2::splat_expr(0_i32));
    }

    **fluence
}
