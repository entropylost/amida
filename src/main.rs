use std::{collections::HashMap, f32::consts::TAU};

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
    let sky_color = FVec3::new(0.3, 0.7, 1.0) * 0.5;
    let sun_color = FVec3::new(1.0, 0.8, 0.4) * 5.0;
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
    let grid_dispatch = [512, 512, 1];

    let app = App::new("Thelema Render", grid_size)
        .scale(4)
        .dpi_override(2.0)
        .agx()
        .init();

    let bounce_cascades = CascadeSettings {
        base_interval: (1.5, 6.0),
        base_probe_spacing: 2.0,
        base_size: CascadeSize {
            probes: Vec2::new(256, 256),
            facings: 16, // 4 normally.
        },
        num_cascades: 5,
        spatial_factor: 1,
        angular_factor: 2,
    };

    let cascades = CascadeSettings {
        base_interval: (0.0, 1.0),
        base_probe_spacing: 1.0,
        base_size: CascadeSize {
            probes: Vec2::new(512, 512),
            facings: 4, // 4 normally.
        },
        num_cascades: 6,
        spatial_factor: 1,
        angular_factor: 2,
    };

    let env_facings = cascades.level_size(cascades.num_cascades).facings;
    assert!(
        env_facings
            == bounce_cascades
                .level_size(bounce_cascades.num_cascades)
                .facings
    );

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
    let bounce_radiance_cascades = RadianceCascades::new(bounce_cascades, &world);

    let reset_radiance_kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        world
            .radiance
            .write(dispatch_id().xy(), emissive.read(dispatch_id().xy()));
    }));
    let update_radiance_kernel = DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..bounce_cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / bounce_cascades.probe_spacing(level).cast_u32(),
                facing: i,
                level,
            });
            *total_radiance += bounce_radiance_cascades.radiance.read(ray);
        }
        let radiance = total_radiance / bounce_cascades.facing_count(level).cast_f32();

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

    let display_kernel = DEVICE.create_kernel::<fn(bool)>(&track!(|diff| {
        app.display().write(
            dispatch_id().xy(),
            world.radiance.read(dispatch_id().xy())
                + if diff {
                    world.diff.read(dispatch_id().xy()).cast_u32().cast_f32() * 5.0
                } else {
                    0.0_f32.expr()
                },
        );
    }));

    let finish_radiance_kernel = DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / cascades.probe_spacing(level).cast_u32(),
                facing: i,
                level,
            });
            *total_radiance += radiance_cascades.radiance.read(ray);
        }
        let radiance = total_radiance / cascades.facing_count(level).cast_f32();
        world.radiance.write(
            dispatch_id().xy(),
            radiance, // world.radiance.read(dispatch_id().xy()), //  * diffuse.read(dispatch_id().xy()),
        );
    }));

    let mut merge_variant = 0;
    let mut num_bounces = 0;
    let mut run_final = true;
    let mut show_diff = false;

    let mut t = 0;

    let mut total_runtime = 0.0;

    #[rustfmt::skip]
    let material_map = vec![
        (MouseButton::Left, (Vec3::splat(0.0), Vec3::splat(1.0), Vec3::splat(f32::INFINITY))),
        (MouseButton::Middle, (Vec3::splat(0.0), Vec3::splat(0.0), Vec3::splat(0.0))),
        (MouseButton::Back, (Vec3::splat(0.0), Vec3::splat(0.0), Vec3::new(0.01, 0.1, 0.1))),
        (MouseButton::Right, (Vec3::splat(15.0), Vec3::splat(0.0), Vec3::splat(0.3))),
    ].into_iter().collect::<HashMap<_, _>>();

    let draw = |pos: Vec2<f32>, r: f32, x: MouseButton| {
        let (emiss, diff, opacity) = material_map[&x];
        draw_kernel.dispatch(grid_dispatch, &pos, &r, &emiss, &diff, &opacity);
    };

    draw(Vec2::new(200.0, 200.0), 40.0, MouseButton::Left);
    draw(Vec2::new(100.0, 50.0), 40.0, MouseButton::Back);
    draw(Vec2::new(400.0, 100.0), 5.0, MouseButton::Right);

    app.run(|rt, scope| {
        if rt.pressed_key(KeyCode::KeyR) {
            rt.begin_recording(None, false);
        }

        t += 1;

        // let pos = Vec2::new(200.0 + 100.0 * (t as f32 / 40.0).cos(), 200.0);

        for button in material_map.keys() {
            if rt.pressed_button(*button) {
                let pos = rt.cursor_position;
                draw(pos, 10.0, *button);
            }
        }

        if rt.just_pressed_key(KeyCode::Enter) {
            merge_variant = (merge_variant + 1) % radiance_cascades.merge_kernel_count();
        }
        if rt.just_pressed_key(KeyCode::KeyL) {
            display_level = (display_level + 1) % cascades.num_cascades;
            println!("Display level: {}", display_level);
        }
        if rt.just_pressed_key(KeyCode::KeyB) {
            num_bounces = (num_bounces + 1) % 4;
            println!("Num bounces: {}", num_bounces);
        }
        if rt.just_pressed_key(KeyCode::KeyF) {
            run_final = !run_final;
            println!("Run final: {}", run_final);
        }
        if rt.just_pressed_key(KeyCode::KeyD) {
            show_diff = !show_diff;
            println!("Show diff: {}", show_diff);
        }

        let timings = (
            reset_radiance_kernel.dispatch_async(grid_dispatch),
            update_diff_kernel.dispatch_async(grid_dispatch),
            (0..num_bounces)
                .map(|_i| {
                    (
                        bounce_radiance_cascades.update(0),
                        update_radiance_kernel.dispatch_async(grid_dispatch, &0),
                        update_diff_kernel.dispatch_async(grid_dispatch),
                    )
                        .chain()
                })
                .collect::<Vec<_>>(),
            run_final.then(|| {
                (
                    radiance_cascades.update(merge_variant),
                    finish_radiance_kernel.dispatch_async(grid_dispatch, &display_level),
                )
                    .chain()
            }),
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

        scope.submit([display_kernel.dispatch_async(grid_dispatch, &show_diff)]);
    });
}
