use std::f32::consts::TAU;

use cascade::{CascadeSettings, CascadeSize, RayLocation, RayLocationComps};
use color::{OpticalDepth, Radiance};
use glam::Vec3 as FVec3;
use luisa::lang::types::vector::{Vec2, Vec3};
use radiance::RadianceCascades;
use sefirot::prelude::*;
use sefirot_testbed::{App, KeyCode, MouseButton};

mod cascade;
mod color;
mod radiance;
mod trace;
mod utils;

struct World {
    size: [u32; 2],
    radiance: Tex2d<Radiance>,
    opacity: Tex2d<OpticalDepth>,
    diff: Tex2d<bool>,
    environment: Buffer<Radiance>,
}
impl World {
    fn width(&self) -> u32 {
        self.size[0]
    }
    fn height(&self) -> u32 {
        self.size[1]
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
            base_interval: (0.0, 11.0),
            base_probe_spacing: 1.0,
            base_size: CascadeSize {
                probes: Vec2::new(512, 512),
                facings: 64,
            },
            num_cascades: 9,
            spatial_factor: 1,
            angular_factor: 1,
        }
    } else {
        CascadeSettings {
            base_interval: (0.0, 1.5),
            base_probe_spacing: 1.0,
            base_size: CascadeSize {
                probes: Vec2::new(512, 512),
                facings: 1, // 4 normally.
            },
            num_cascades: 6,
            spatial_factor: 1,
            angular_factor: 2,
        }
    };

    let env_facings = cascades.level_size(cascades.num_cascades).facings;

    let world = World {
        size: grid_size,
        radiance: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        opacity: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        diff: DEVICE.create_tex2d(PixelStorage::Byte1, grid_size[0], grid_size[1], 1),
        environment: DEVICE.create_buffer_from_fn(env_facings as usize, |i| {
            let angle = TAU - i as f32 / env_facings as f32 * TAU;
            Vec3::from(skylight(angle))
        }),
    };
    let emissive =
        DEVICE.create_tex2d::<Vec3<f32>>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);
    let diffuse =
        DEVICE.create_tex2d::<Vec3<f32>>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);

    // let background =
    //     DEVICE.create_tex2d::<Vec3<f32>>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);

    let radiance_cascades = RadianceCascades::new(cascades, &world);

    let reset_radiance_kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        world
            .radiance
            .write(dispatch_id().xy(), emissive.read(dispatch_id().xy()));
    }));
    let update_radiance_kernel = DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / (1_u32 << (cascades.spatial_factor * level)),
                facing: i,
                level,
            });
            *total_radiance += radiance_cascades.radiance.read(ray);
        }
        let radiance = total_radiance / cascades.facing_count(level).cast_f32();

        let diffuse = diffuse.read(dispatch_id().xy());
        let emissive = emissive.read(dispatch_id().xy());

        world
            .radiance
            .write(dispatch_id().xy(), radiance * diffuse + emissive);
    }));

    let update_diff_kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        let pos = dispatch_id().xy();
        let diff = false.var();
        let radiance = world.radiance.read(pos);
        let depth = world.opacity.read(pos);
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
                let neighbor_radiance = world.radiance.read(neighbor.cast_u32());
                let neighbor_depth = world.opacity.read(neighbor.cast_u32());
                if (neighbor_radiance != radiance).any() || (neighbor_depth != depth).any() {
                    *diff = true;
                    break;
                }
            }
        }
        world.diff.write(pos, **diff);
    }));

    let copy_kernel =
        DEVICE.create_kernel::<fn(Vec3<f32>, Tex2d<Vec3<f32>>)>(&track!(|src, dst| {
            dst.write(dispatch_id().xy(), src);
        }));

    let draw_kernel = DEVICE.create_kernel::<fn(Vec2<f32>, f32, Vec3<f32>, Vec3<f32>, Vec3<f32>)>(
        &track!(|pos, radius, emiss, diff, opacity| {
            if (dispatch_id().xy().cast_f32() - pos).length() < radius {
                emissive.write(dispatch_id().xy(), emiss);
                diffuse.write(dispatch_id().xy(), diff);
                world.opacity.write(dispatch_id().xy(), opacity);
            }
        }),
    );

    let mut display_level = 0;

    let display_kernel = DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / (1_u32 << (cascades.spatial_factor * level)),
                facing: i,
                level,
            });
            *total_radiance += radiance_cascades.radiance.read(ray);
        }
        let radiance = total_radiance / cascades.facing_count(level).cast_f32();
        app.display().write(
            dispatch_id().xy(),
            radiance, //  * diffuse.read(dispatch_id().xy()),
        );
    }));

    let mut merge_variant = 1;

    let mut t = 0;

    // copy_kernel.dispatch([512, 512, 1], &Vec3::splat(1.0), &background);
    copy_kernel.dispatch([512, 512, 1], &Vec3::splat(0.0), &world.opacity);

    draw_kernel.dispatch(
        [512, 512, 1],
        &Vec2::new(256.0, 256.0),
        &40.0,
        &Vec3::splat(0.0),
        &Vec3::splat(0.7),
        &Vec3::splat(f32::INFINITY),
    );

    draw_kernel.dispatch(
        [512, 512, 1],
        &Vec2::new(400.0, 256.0),
        &10.0,
        &Vec3::splat(10.0),
        &Vec3::splat(0.0),
        &Vec3::splat(0.3),
    );

    draw_kernel.dispatch(
        [512, 512, 1],
        &Vec2::new(200.0, 100.0),
        &40.0,
        &Vec3::splat(0.0),
        &Vec3::splat(0.0),
        &Vec3::new(0.01, 0.1, 0.1),
    );

    let mut total_runtime = 0.0;

    app.run(|rt, scope| {
        if rt.pressed_key(KeyCode::KeyR) {
            rt.begin_recording(None, false);
        }

        t += 1;

        // let pos = Vec2::new(200.0 + 100.0 * (t as f32 / 40.0).cos(), 200.0);

        if rt.pressed_button(MouseButton::Left) {
            let pos = rt.cursor_position;
            draw_kernel.dispatch(
                [512, 512, 1],
                &pos,
                &10.0,
                &Vec3::splat(0.0),
                &Vec3::splat(0.7),
                &Vec3::splat(f32::INFINITY),
            );
        }
        if rt.pressed_button(MouseButton::Middle) {
            let pos = rt.cursor_position;
            draw_kernel.dispatch(
                [512, 512, 1],
                &pos,
                &10.0,
                &Vec3::splat(0.0),
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
                &Vec3::splat(10.0),
                &Vec3::splat(0.0),
                &Vec3::splat(0.3),
            );
        }

        if rt.just_pressed_key(KeyCode::Enter) {
            merge_variant = (merge_variant + 1) % radiance_cascades.merge_kernel_count();
        }
        if rt.just_pressed_key(KeyCode::KeyL) {
            display_level = (display_level + 1) % cascades.num_cascades;
        }

        let timings = (
            reset_radiance_kernel.dispatch([512, 512, 1]),
            update_diff_kernel.dispatch([512, 512, 1]),
            radiance_cascades.update(merge_variant),
            // update_radiance_kernel.dispatch([512, 512, 1], &1),
            // radiance_cascades.update(merge_variant),
        )
            .chain()
            .execute_timed();
        if rt.just_pressed_key(KeyCode::Space) {
            println!("{:?}", timings);
        }
        total_runtime += timings.iter().map(|(_, t)| t).sum::<f32>();
        if t % 100 == 0 {
            println!("Total runtime: {}", total_runtime / 100.0);
            total_runtime = 0.0;
        }

        scope.submit([display_kernel.dispatch_async([512, 512, 1], &display_level)]);
    });
}
