use super::*;
use color::*;

pub type Interval = Vec2<f32>;

#[cfg(not(any(feature = "block16", feature = "block64")))]
pub type BlockType = bool;
#[cfg(feature = "block16")]
pub type BlockType = u16;
#[cfg(feature = "block64")]
pub type BlockType = u64;

pub trait Block: Value {
    type Storage: IoTexel;
    const STORAGE_FORMAT: PixelStorage;
    const SIZE: u32;

    fn read(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>) -> Expr<Self>;
    fn write(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>, value: Expr<Self>);
    fn get(this: Expr<Self>, offset: Expr<Vec2<u32>>) -> Expr<bool>;
    fn set(this: Var<Self>, offset: Expr<Vec2<u32>>);
    fn is_empty(this: Expr<Self>) -> Expr<bool>;
    fn empty() -> Self;
}
impl Block for bool {
    type Storage = bool;
    const STORAGE_FORMAT: PixelStorage = PixelStorage::Byte1;
    const SIZE: u32 = 1;

    fn read(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>) -> Expr<Self> {
        storage.read(offset)
    }
    fn write(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>, value: Expr<Self>) {
        storage.write(offset, value);
    }
    fn get(this: Expr<Self>, _offset: Expr<Vec2<u32>>) -> Expr<bool> {
        this
    }
    #[tracked]
    fn set(this: Var<Self>, _offset: Expr<Vec2<u32>>) {
        *this = true;
    }
    #[tracked]
    fn is_empty(this: Expr<Self>) -> Expr<bool> {
        !this
    }
    fn empty() -> Self {
        false
    }
}
impl Block for u16 {
    type Storage = u32;
    const STORAGE_FORMAT: PixelStorage = PixelStorage::Short1;
    const SIZE: u32 = 4;

    fn read(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>) -> Expr<Self> {
        storage.read(offset).cast_u16()
    }
    fn write(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>, value: Expr<Self>) {
        storage.write(offset, value.cast_u32());
    }
    #[tracked]
    fn get(this: Expr<Self>, offset: Expr<Vec2<u32>>) -> Expr<bool> {
        this & (1 << (offset.x + offset.y * 4).cast_u16()) != 0
    }
    #[tracked]
    fn set(this: Var<Self>, offset: Expr<Vec2<u32>>) {
        *this |= 1 << (offset.x + offset.y * 4).cast_u16();
    }
    #[tracked]
    fn is_empty(this: Expr<Self>) -> Expr<bool> {
        this == 0
    }
    fn empty() -> Self {
        0
    }
}
impl Block for u64 {
    type Storage = Vec2<u32>;
    const STORAGE_FORMAT: PixelStorage = PixelStorage::Int2;
    const SIZE: u32 = 8;

    #[tracked]
    fn read(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>) -> Expr<Self> {
        let v = storage.read(offset);
        v.x.cast_u64() | (v.y.cast_u64() << 32)
    }
    #[tracked]
    fn write(storage: &Tex2dView<Self::Storage>, offset: Expr<Vec2<u32>>, value: Expr<Self>) {
        storage.write(
            offset,
            Vec2::expr(value.cast_u32(), (value >> 32).cast_u32()),
        );
    }
    #[tracked]
    fn get(this: Expr<Self>, offset: Expr<Vec2<u32>>) -> Expr<bool> {
        this & (1 << (offset.x + offset.y * 8).cast_u64()) != 0
    }
    #[tracked]
    fn set(this: Var<Self>, offset: Expr<Vec2<u32>>) {
        *this |= 1 << (offset.x + offset.y * 4).cast_u64();
    }
    #[tracked]
    fn is_empty(this: Expr<Self>) -> Expr<bool> {
        this == 0
    }
    fn empty() -> Self {
        0
    }
}

pub struct TraceWorld<B: Block = BlockType> {
    pub size: [u32; 2],
    pub radiance: Tex2dView<Radiance>,
    pub opacity: Tex2dView<Opacity>,
    pub environment: BufferView<Radiance>,
    pub difference: Tex2dView<B::Storage>,
}
impl<B: Block> TraceWorld<B> {
    pub fn width(&self) -> u32 {
        self.size[0]
    }
    pub fn height(&self) -> u32 {
        self.size[1]
    }
}

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
#[cfg(any(feature = "block16", feature = "block64"))]
pub fn trace_radiance(
    world: &TraceWorld,
    ray_start: Expr<Vec2<f32>>,
    ray_dir: Expr<Vec2<f32>>,
    interval: Expr<Interval>,
) -> Expr<Fluence> {
    trace_radiance_block(world, ray_start, ray_dir, interval)
}
#[cfg(not(any(feature = "block16", feature = "block64")))]
pub fn trace_radiance(
    world: &TraceWorld,
    ray_start: Expr<Vec2<f32>>,
    ray_dir: Expr<Vec2<f32>>,
    interval: Expr<Interval>,
) -> Expr<Fluence> {
    trace_radiance_single(world, ray_start, ray_dir, interval)
}

#[tracked]
fn trace_radiance_block<B: Block>(
    world: &TraceWorld<B>,
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

    let pos = ray_start.floor().cast_i32().var();
    let block_pos = (pos / B::SIZE as i32).var();

    let delta_dist = inv_dir.abs();
    let block_delta_dist = delta_dist * B::SIZE as f32;

    let ray_step = ray_dir.signum().cast_i32();
    let side_dist =
        (ray_dir.signum() * (pos.cast_f32() - ray_start) + ray_dir.signum() * 0.5 + 0.5)
            * delta_dist;
    let side_dist = side_dist.var();
    let block_side_dist = (ray_dir.signum() * (block_pos.cast_f32() - ray_start / B::SIZE as f32)
        + ray_dir.signum() * 0.5
        + 0.5)
        * block_delta_dist;
    let block_side_dist = block_side_dist.var();

    let block_offset = (ray_dir > 0.0).select(
        Vec2::splat_expr(0_i32),
        Vec2::splat_expr(B::SIZE as i32 - 1),
    );

    let block = B::read(&world.difference, block_pos.cast_u32()).var();

    let interval_size = interval.y - interval.x;

    let last_t = 0.0_f32.var();

    let fluence = Fluence::transparent().var();

    for _i in 0_u32.expr()..1000_u32.expr() {
        if B::is_empty(**block) || (pos / B::SIZE as i32 != block_pos).any() {
            let t = block_side_dist.reduce_min();
            let mask = block_side_dist <= block_side_dist.yx();

            *block_side_dist += mask.select(block_delta_dist, Vec2::splat_expr(0.0));
            *block_pos += mask.select(ray_step, Vec2::splat_expr(0));

            *block = B::read(&world.difference, block_pos.cast_u32());

            if !B::is_empty(**block) {
                *pos = mask.select(
                    block_pos * B::SIZE as i32 + block_offset,
                    (t * ray_dir + ray_start).floor().cast_i32(),
                );
                *side_dist = (ray_dir.signum() * (pos.cast_f32() - ray_start)
                    + ray_dir.signum() * 0.5
                    + 0.5)
                    * delta_dist;
            } else {
                continue;
            }
        }

        let next_t = side_dist.reduce_min();

        if B::get(**block, pos.cast_u32() % B::SIZE) || next_t >= interval_size {
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
        *pos += mask.select(ray_step, Vec2::splat_expr(0));
    }

    **fluence
}

#[tracked]
fn trace_radiance_single(
    world: &TraceWorld<bool>,
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

    let pos = ray_start.floor().cast_i32().var();

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

        if world.difference.read(pos.cast_u32()) || next_t >= interval_size {
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
