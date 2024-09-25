use std::f32::consts::TAU;

use glam::Vec3 as FVec3;
use luisa::lang::types::vector::{Vec2, Vec3};
use sefirot::prelude::*;
use sefirot_testbed::{App, KeyCode, MouseButton};

type Interval = Vec2<f32>;
// w is transmittance.

// https://github.com/markjarzynski/PCG3D/blob/master/pcg3d.hlsl
#[tracked]
fn pcg3d(v: Expr<Vec3<u32>>) -> Expr<Vec3<u32>> {
    let v = v.var();
    *v = v * 1664525u32 + 1013904223u32;

    *v.x += v.y * v.z;
    *v.y += v.z * v.x;
    *v.z += v.x * v.y;

    *v ^= v >> 16u32;

    *v.x += v.y * v.z;
    *v.y += v.z * v.x;
    *v.z += v.x * v.y;

    **v
}

#[tracked]
fn pcg3df(v: Expr<Vec3<u32>>) -> Expr<Vec3<f32>> {
    pcg3d(v).cast_f32() / u32::MAX as f32
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
struct Color {
    radiance: Vec3<f32>,
    opacity: Vec3<f32>,
}
impl ColorExpr {
    #[tracked]
    fn as_fluence(&self, segment_size: Expr<f32>) -> Expr<Fluence> {
        let color = self.self_;
        Fluence::from_comps_expr(FluenceComps {
            radiance: color.radiance * (1.0 - (-color.opacity * segment_size).exp()),
            transmittance: (-color.opacity * segment_size).exp(),
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
struct Fluence {
    radiance: Vec3<f32>,
    transmittance: Vec3<f32>,
}

impl Fluence {
    fn transparent() -> Self {
        Self {
            radiance: Vec3::splat(0.0),
            transmittance: Vec3::splat(1.0),
        }
    }
}

impl FluenceExpr {
    #[tracked]
    fn over(self, far: Expr<Fluence>) -> Expr<Fluence> {
        let near = self.self_;
        Fluence::from_comps_expr(FluenceComps {
            radiance: near.radiance + near.transmittance * far.radiance,
            transmittance: near.transmittance * far.transmittance,
        })
    }
    #[tracked]
    fn over_color(self, far: Expr<Vec3<f32>>) -> Expr<Vec3<f32>> {
        let near = self.self_;
        near.radiance + near.transmittance * far
    }
}

const TRANSMITTANCE_CUTOFF: f32 = 0.01;

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
fn trace_radiance(
    color_texture: &Tex2dView<Vec3<f32>>,
    opacity_texture: &Tex2dView<Vec3<f32>>,
    diff_texture: &Tex2dView<bool>,
    ray_start: Expr<Vec2<f32>>,
    ray_dir: Expr<Vec2<f32>>,
    interval: Expr<Interval>,
) -> Expr<Fluence> {
    assert_eq!(color_texture.size(), opacity_texture.size());

    let inv_dir = (ray_dir + f32::EPSILON).recip();

    let interval = intersect_intervals(
        interval,
        aabb_intersect(
            ray_start,
            inv_dir,
            Vec2::splat_expr(0.01),
            Vec2::expr(
                color_texture.size()[0] as f32,
                color_texture.size()[1] as f32,
            ) - Vec2::splat_expr(0.01),
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

    let radiance = Fluence::transparent().var();

    for _i in 0_u32.expr()..1000_u32.expr() {
        let next_t = side_dist.reduce_min();

        if diff_texture.read(pos.cast_u32()) {
            let segment_size = luisa::min(next_t, interval_size) - last_t;
            let color = color_texture.read(pos.cast_u32());
            let opacity = opacity_texture.read(pos.cast_u32());
            *radiance = radiance.over(
                Color::from_comps_expr(ColorComps {
                    radiance: color,
                    opacity,
                })
                .as_fluence(segment_size),
            );

            *last_t = next_t;

            if (radiance.transmittance < TRANSMITTANCE_CUTOFF).any() {
                break;
            }
        }

        if next_t >= interval_size {
            break;
        }

        let mask = side_dist <= side_dist.yx();

        *side_dist += mask.select(delta_dist, Vec2::splat_expr(0.0));
        *pos += mask.select(ray_step, Vec2::splat_expr(0_i32));
    }

    **radiance
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
struct BilinearSamples {
    base_index: Vec2<u32>,
    fract: Vec2<f32>,
}
impl BilinearSamplesExpr {
    #[tracked]
    fn sample(&self, index: Expr<u32>) -> (Expr<Vec2<u32>>, Expr<f32>) {
        let fract = self.fract;
        let weights = <[_; 4]>::from_elems_expr([
            (1.0 - fract.x) * (1.0 - fract.y),
            fract.x * (1.0 - fract.y),
            (1.0 - fract.x) * fract.y,
            fract.x * fract.y,
        ]);
        let indices = [
            Vec2::new(0, 0),
            Vec2::new(1, 0),
            Vec2::new(0, 1),
            Vec2::new(1, 1),
        ]
        .expr();
        (indices[index] + self.base_index, weights[index])
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CascadeSettings {
    base_interval_size: f32,
    base_probe_spacing: f32,
    base_size: CascadeSize,
    num_cascades: u32,
    spatial_factor: u32, // default 1
    angular_factor: u32, // default 2
}
impl CascadeSettings {
    #[tracked]
    fn bilinear_samples(
        &self,
        probe: Expr<Vec2<u32>>,
        next_level: Expr<u32>,
    ) -> Expr<BilinearSamples> {
        let next_level_probe_location = ((probe.cast_f32() + 0.5) / self.spacing() as f32) - 0.5;
        let next_level_probe_location = next_level_probe_location.clamp(
            0.0,
            (self.base_size.probes >> (next_level * self.spatial_factor)).cast_f32() - 1.0,
        );
        let base_index = next_level_probe_location.floor().cast_u32();
        let fract = next_level_probe_location.fract();
        BilinearSamples::from_comps_expr(BilinearSamplesComps { base_index, fract })
    }
    #[tracked]
    fn dir_of(&self, index: Expr<u32>, level: Expr<u32>) -> Expr<Vec2<f32>> {
        let angle = (index.cast_f32() + 0.5_f32)
            / (self.base_size.directions << (self.angular_factor * level)).cast_f32()
            * TAU;
        Vec2::expr(angle.cos(), angle.sin())
    }
    #[tracked]
    fn probe_location(&self, index: Expr<Vec2<u32>>, level: Expr<u32>) -> Expr<Vec2<f32>> {
        (index.cast_f32() + 0.5) * self.probe_spacing(level)
    }
    #[tracked]
    fn probe_spacing(&self, level: Expr<u32>) -> Expr<f32> {
        self.base_probe_spacing * (1 << (self.spatial_factor * level)).cast_f32()
    }
    fn spacing(&self) -> u32 {
        1 << self.spatial_factor
    }
    fn branches(&self) -> u32 {
        1 << self.angular_factor
    }
    #[tracked]
    fn base_interval_size(&self) -> f32 {
        self.base_interval_size
    }
    #[tracked]
    fn interval_end(&self, level: Expr<u32>) -> Expr<f32> {
        // TODO: Change?
        self.base_interval_size() * (1_u32 << (self.angular_factor * level)).cast_f32()
    }
    #[tracked]
    fn interval(&self, level: Expr<u32>) -> Expr<Interval> {
        Vec2::expr(
            if level == 0 {
                0.0_f32.expr()
            } else {
                self.interval_end(level - 1)
            },
            self.interval_end(level),
        )
    }
    fn level_size(&self, level: u32) -> CascadeSize {
        CascadeSize {
            probes: Vec2::new(
                self.base_size.probes.x >> (level * self.spatial_factor), // .max(1)
                self.base_size.probes.y >> (level * self.spatial_factor), // TODO: That'd break cascade_total_size.
            ),
            directions: self.base_size.directions << (level * self.angular_factor),
        }
    }
    #[tracked]
    fn level_size_expr(&self, level: Expr<u32>) -> Expr<CascadeSize> {
        CascadeSize::from_comps_expr(CascadeSizeComps {
            probes: self.base_size.probes >> (level * self.spatial_factor),
            directions: self.base_size.directions << (level * self.angular_factor),
        })
    }
    fn cascade_total_size(&self) -> u32 {
        assert!(self.spatial_factor * 2 >= self.angular_factor); // So they cancel out. Still not quite exact.
        self.base_size.probes.x * self.base_size.probes.y * self.base_size.directions
    }
}

struct CascadeStorage<T: Value> {
    settings: CascadeSettings,
    buffer: Buffer<T>,
}
impl<T: Value> CascadeStorage<T> {
    fn new(settings: CascadeSettings) -> Self {
        let buffer =
            DEVICE.create_buffer((settings.cascade_total_size() * settings.num_cascades) as usize);
        Self { settings, buffer }
    }
    #[tracked]
    fn to_index(&self, ray: Expr<RayLocation>) -> Expr<u32> {
        let cascade_total_size = self.settings.cascade_total_size();
        let cascade_size = self.settings.level_size_expr(ray.cascade);
        // Store rays with same direction near to each other.
        let linear_index = ray.probe.x + ray.probe.y * cascade_size.probes.x;
        // Other way seems to run slightly slower.
        ray.cascade * cascade_total_size + linear_index * cascade_size.directions + ray.direction
    }
    #[tracked]
    fn read(&self, ray: Expr<RayLocation>) -> Expr<T> {
        // let a = ray.cascade < self.settings.num_cascades;
        // lc_assert!(a);
        // let b = ray.probe.x
        //     < self.settings.base_size.probes.x >> (ray.cascade * self.settings.spatial_factor);
        // lc_assert!(b);
        // let c = ray.probe.y
        //     < self.settings.base_size.probes.y >> (ray.cascade * self.settings.spatial_factor);
        // lc_assert!(c);
        // let d = ray.direction
        //     < self.settings.base_size.directions << (ray.cascade * self.settings.angular_factor);
        // lc_assert!(d);

        self.buffer.read(self.to_index(ray))
    }
    fn write(&self, ray: Expr<RayLocation>, value: Expr<T>) {
        self.buffer.write(self.to_index(ray), value);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Value)]
struct CascadeSize {
    probes: Vec2<u32>,
    directions: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Value)]
struct RayLocation {
    probe: Vec2<u32>,
    direction: u32,
    cascade: u32,
}

struct RadianceCascades {
    settings: CascadeSettings,
    radiance: CascadeStorage<Vec3<f32>>,
    merge_kernels: [luisa::runtime::Kernel<fn(u32)>; 3],
}

impl RadianceCascades {
    fn new(
        settings: CascadeSettings,
        color_texture: Tex2dView<Vec3<f32>>,
        opacity_texture: Tex2dView<Vec3<f32>>,
        diff_texture: Tex2dView<bool>,
        environment: BufferView<Vec3<f32>>,
    ) -> Self {
        let radiance = CascadeStorage::new(settings);

        let merge_kernels = [
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                let direction = dispatch_id().z;
                let probe = dispatch_id().xy();
                let ray = RayLocation::from_comps_expr(RayLocationComps {
                    probe,
                    direction,
                    cascade: level,
                });

                let probe_pos = settings.probe_location(probe, level);

                let ray_dir = settings.dir_of(direction, level);

                let interval = settings.interval(level);

                let next_cascade = level + 1;
                let samples = settings.bilinear_samples(probe, next_cascade);

                let rand = pcg3df(dispatch_id() + Vec3::expr(0, 0, level << 16));
                let next_probe = samples.base_index + (rand.xy() < samples.fract).cast_u32();
                let next_probe_pos = settings.probe_location(next_probe, next_cascade);
                let ray_start = probe_pos + ray_dir * interval.x;
                let ray_end = next_probe_pos + ray_dir * interval.y;
                let ray_fluence = trace_radiance(
                    &color_texture,
                    &opacity_texture,
                    &diff_texture,
                    ray_start,
                    (ray_end - ray_start).normalize(),
                    Vec2::expr(0.0, (ray_end - ray_start).length()),
                );

                let total_radiance = Vec3::splat(0.0_f32).var();
                for i in 0_u32.expr()..settings.branches().expr() {
                    let next_direction = direction * settings.branches() + i;

                    let next_ray = RayLocation::from_comps_expr(RayLocationComps {
                        probe: next_probe,
                        direction: next_direction,
                        cascade: next_cascade,
                    });

                    let next_radiance = if next_cascade < settings.num_cascades {
                        radiance.read(next_ray)
                    } else {
                        environment.read(next_direction)
                    };

                    *total_radiance += next_radiance;
                }
                let avg_radiance = total_radiance / settings.branches() as f32;
                radiance.write(ray, ray_fluence.over_color(avg_radiance));
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                set_block_size([1, 1, 32]);
                let direction = dispatch_id().z;
                let probe = dispatch_id().xy();
                let ray = RayLocation::from_comps_expr(RayLocationComps {
                    probe,
                    direction,
                    cascade: level,
                });

                let probe_pos = settings.probe_location(probe, level);

                let ray_dir = settings.dir_of(direction, level);

                let interval = settings.interval(level);

                let next_cascade = level + 1;
                let samples = settings.bilinear_samples(probe, next_cascade);

                let total_radiance = Vec3::splat(0.0_f32).var();
                for i in 0_u32.expr()..settings.branches().expr() {
                    let next_direction = direction * settings.branches() + i;
                    let rand = pcg3df(dispatch_id() + Vec3::expr(0, i << 16, level << 16));
                    let next_probe = samples.base_index + (rand.xy() < samples.fract).cast_u32();
                    let next_probe_pos = settings.probe_location(next_probe, next_cascade);

                    let next_ray_dir = settings.dir_of(next_direction, next_cascade);

                    let ray_start = probe_pos + ray_dir * interval.x;
                    let ray_end = next_probe_pos + next_ray_dir * interval.y;

                    let ray_fluence = trace_radiance(
                        &color_texture,
                        &opacity_texture,
                        &diff_texture,
                        ray_start,
                        (ray_end - ray_start).normalize(),
                        Vec2::expr(0.0, (ray_end - ray_start).length()),
                    );

                    let next_ray = RayLocation::from_comps_expr(RayLocationComps {
                        probe: next_probe,
                        direction: next_direction,
                        cascade: next_cascade,
                    });

                    let next_radiance = if next_cascade < settings.num_cascades {
                        radiance.read(next_ray)
                    } else {
                        environment.read(next_direction)
                    };

                    let merged_radiance = ray_fluence.over_color(next_radiance);
                    *total_radiance += merged_radiance;
                }
                let avg_radiance = total_radiance / settings.branches() as f32;
                radiance.write(ray, avg_radiance);
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                let direction = dispatch_id().z;
                let probe = dispatch_id().xy();
                let ray = RayLocation::from_comps_expr(RayLocationComps {
                    probe,
                    direction,
                    cascade: level,
                });

                let probe_pos = settings.probe_location(probe, level);

                let ray_dir = settings.dir_of(direction, level);

                let interval = settings.interval(level);

                let next_cascade = level + 1;
                let samples = settings.bilinear_samples(probe, next_cascade);

                let out_radiance = Vec3::splat(0.0_f32).var();

                for probe_offset in 0_u32.expr()..4_u32.expr() {
                    let (next_probe, weight) = samples.sample(probe_offset);
                    if (next_probe >= settings.level_size_expr(next_cascade).probes).any() {
                        continue;
                    }
                    let next_probe_pos = settings.probe_location(next_probe, next_cascade);
                    let ray_start = probe_pos + ray_dir * interval.x;
                    let ray_end = next_probe_pos + ray_dir * interval.y;

                    let ray_fluence = trace_radiance(
                        &color_texture,
                        &opacity_texture,
                        &diff_texture,
                        ray_start,
                        (ray_end - ray_start).normalize(),
                        Vec2::expr(0.0, (ray_end - ray_start).length()),
                    );

                    let total_radiance = Vec3::splat(0.0_f32).var();

                    for i in 0_u32.expr()..settings.branches().expr() {
                        let next_direction = direction * settings.branches() + i;
                        let next_ray = RayLocation::from_comps_expr(RayLocationComps {
                            probe: next_probe,
                            direction: next_direction,
                            cascade: next_cascade,
                        });

                        let next_radiance = if next_cascade < settings.num_cascades {
                            radiance.read(next_ray)
                        } else {
                            environment.read(next_direction)
                        };
                        *total_radiance += next_radiance;
                    }

                    let merged_radiance =
                        ray_fluence.over_color(total_radiance / settings.branches() as f32);
                    *out_radiance += merged_radiance * weight;
                }

                radiance.write(ray, **out_radiance);
            })),
        ];

        Self {
            settings,
            radiance,
            merge_kernels,
        }
    }
    fn update(&self, variant: usize) -> impl AsNodes {
        let mut commands = vec![];
        for level in (0..self.settings.num_cascades).rev() {
            let level_size = self.settings.level_size(level);
            commands.push(
                self.merge_kernels[variant]
                    .dispatch_async(
                        [
                            level_size.probes.x,
                            level_size.probes.y,
                            level_size.directions,
                        ],
                        &level,
                    )
                    .debug(format!("merge level {}", level)),
            );
        }
        commands.chain()
    }
}

fn skylight(angle: f32) -> FVec3 {
    let sky_color = FVec3::new(0.3, 0.7, 1.0);
    let sun_color = FVec3::new(1.0, 1.0, 0.8) * 3.0;
    let sun_size = 0.3;
    let sun_angle = 1.0;

    let sun_color = if (angle - sun_angle).abs() < sun_size {
        sun_color
    } else {
        FVec3::ZERO
    };
    sun_color + sky_color * angle.sin().max(0.0).powi(2)
}

fn main() {
    let grid_size = [512, 512];

    let app = App::new("Thelema Render", grid_size)
        .scale(4)
        .dpi_override(2.0)
        .agx()
        .init();

    let cascades = if false {
        CascadeSettings {
            base_interval_size: 11.0,
            base_probe_spacing: 1.0,
            base_size: CascadeSize {
                probes: Vec2::new(512, 512),
                directions: 64,
            },
            num_cascades: 9,
            spatial_factor: 1,
            angular_factor: 1,
        }
    } else {
        CascadeSettings {
            base_interval_size: 1.2,
            base_probe_spacing: 1.0,
            base_size: CascadeSize {
                probes: Vec2::new(512, 512),
                directions: 1, // 4 normally.
            },
            num_cascades: 6,
            spatial_factor: 1,
            angular_factor: 2,
        }
    };

    let color_texture = DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1);
    let opacity_texture = DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1);
    let diff_texture = DEVICE.create_tex2d(PixelStorage::Byte1, grid_size[0], grid_size[1], 1);

    let env_dirs = cascades.level_size(cascades.num_cascades).directions;
    let environment_buffer = DEVICE.create_buffer_from_fn(env_dirs as usize, |i| {
        let angle = TAU - i as f32 / env_dirs as f32 * TAU;
        Vec3::from(skylight(angle))
    });

    let radiance_cascades = RadianceCascades::new(
        cascades,
        color_texture.view(0),
        opacity_texture.view(0),
        diff_texture.view(0),
        environment_buffer.view(..),
    );

    let update_diff_kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        let pos = dispatch_id().xy();
        let diff = false.var();
        let color = color_texture.read(pos);
        let opacity = opacity_texture.read(pos);
        for i in 0_u32.expr()..4_u32.expr() {
            let offset = [
                Vec2::new(1, 0),
                Vec2::new(-1, 0),
                Vec2::new(0, 1),
                Vec2::new(0, -1),
            ]
            .expr()[i];
            let neighbor = pos.cast_i32() + offset;
            if (neighbor >= 0).all() && (neighbor < Vec2::from(grid_size).expr().cast_i32()).all() {
                let neighbor_color = color_texture.read(neighbor.cast_u32());
                let neighbor_opacity = opacity_texture.read(neighbor.cast_u32());
                if (neighbor_color != color).any() || (neighbor_opacity != opacity).any() {
                    *diff = true;
                    break;
                }
            }
        }
        diff_texture.write(pos, **diff);
    }));

    let draw_kernel = DEVICE.create_kernel::<fn(Vec2<f32>, f32, Vec3<f32>, Vec3<f32>)>(&track!(
        |pos, radius, color, opacity| {
            if (dispatch_id().xy().cast_f32() - pos).length() < radius {
                color_texture.write(dispatch_id().xy(), color);
                opacity_texture.write(dispatch_id().xy(), opacity);
            }
        }
    ));

    let display_kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..radiance_cascades.settings.base_size.directions.expr() {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy(),
                direction: i,
                cascade: 0_u32.expr(),
            });
            *total_radiance += radiance_cascades.radiance.read(ray);
        }
        let radiance = total_radiance / radiance_cascades.settings.base_size.directions as f32;
        let opacity = opacity_texture.read(dispatch_id().xy());
        let color = color_texture.read(dispatch_id().xy());
        app.display().write(
            dispatch_id().xy(),
            color * opacity + radiance * (1.0 - opacity),
        );
    }));

    let mut merge_variant = 1;

    let mut t = 0;

    draw_kernel.dispatch(
        [512, 512, 1],
        &Vec2::new(256.0, 256.0),
        &40.0,
        &Vec3::splat(0.0),
        &Vec3::splat(1.0),
    );

    draw_kernel.dispatch(
        [512, 512, 1],
        &Vec2::new(400.0, 256.0),
        &10.0,
        &Vec3::splat(10.0),
        &Vec3::splat(1.0),
    );

    draw_kernel.dispatch(
        [512, 512, 1],
        &Vec2::new(200.0, 100.0),
        &40.0,
        &Vec3::splat(0.0),
        &Vec3::new(0.01, 0.1, 0.1),
    );

    update_diff_kernel.dispatch([512, 512, 1]);

    app.run(|rt, scope| {
        if rt.pressed_key(KeyCode::KeyR) {
            rt.begin_recording(None, false);
        }

        t += 1;

        let pos = Vec2::new(200.0 + 100.0 * (t as f32 / 40.0).cos(), 200.0);

        if rt.pressed_button(MouseButton::Left) {
            let pos = rt.cursor_position;
            draw_kernel.dispatch(
                [512, 512, 1],
                &pos,
                &10.0,
                &Vec3::splat(1.0),
                &Vec3::splat(0.3),
            );
        }
        if rt.pressed_button(MouseButton::Middle) {
            let pos = rt.cursor_position;
            draw_kernel.dispatch(
                [512, 512, 1],
                &pos,
                &10.0,
                &Vec3::splat(0.0),
                &Vec3::new(0.01, 0.1, 0.1),
            );
        }

        if rt.pressed_button(MouseButton::Right) {
            let pos = rt.cursor_position;
            draw_kernel.dispatch(
                [512, 512, 1],
                &pos,
                &10.0,
                &Vec3::splat(0.0),
                &Vec3::splat(1.0),
            );
        }

        if rt.just_pressed_key(KeyCode::Enter) {
            merge_variant = (merge_variant + 1) % 3;
        }

        if rt.just_pressed_key(KeyCode::Space) {
            let timings = radiance_cascades.update(merge_variant).execute_timed();
            // println!("{:?}", timings);
            println!("Total: {:?}", timings.iter().map(|(_, t)| t).sum::<f32>());
        }

        scope.submit([display_kernel.dispatch_async([512, 512, 1])]);
    });
}
