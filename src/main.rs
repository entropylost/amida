use std::{collections::HashMap, f32::consts::TAU};

use cascade::{CascadeSettings, CascadeSize, RayLocation, RayLocationComps};
use color::{Diffuse, Opacity, Radiance};
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
    emissive: Tex2d<Radiance>,
    diffuse: Tex2d<Diffuse>,
    opacity: Tex2d<Opacity>,
    display_emissive: Tex2d<Radiance>,
    display_diffuse: Tex2d<Diffuse>,
    display_opacity: Tex2d<Opacity>,
    environment: Buffer<Radiance>,
}

struct TraceWorld {
    size: [u32; 2],
    radiance: Tex2dView<Radiance>,
    opacity: Tex2dView<Opacity>,
    difference: Tex2dView<bool>,
    environment: BufferView<Radiance>,
}
impl TraceWorld {
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
    let bounce_env_facings = bounce_cascades
        .level_size(bounce_cascades.num_cascades)
        .facings;
    assert_eq!(bounce_env_facings % env_facings, 0);
    let env_facing_ratio = env_facings / bounce_env_facings;

    let world = World {
        size: grid_size,
        emissive: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        diffuse: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        opacity: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        display_emissive: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        display_diffuse: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        display_opacity: DEVICE.create_tex2d(PixelStorage::Float4, grid_size[0], grid_size[1], 1),
        environment: DEVICE.create_buffer(env_facings as usize),
    };

    let radiance =
        DEVICE.create_tex2d::<Radiance>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);
    let bounce_environment = DEVICE.create_buffer(bounce_env_facings as usize);
    let difference =
        DEVICE.create_tex2d::<bool>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);

    let bounce_radiance_cascades = RadianceCascades::new(
        bounce_cascades,
        &TraceWorld {
            size: grid_size,
            radiance: radiance.view(0),
            opacity: world.opacity.view(0),
            difference: difference.view(0),
            environment: bounce_environment.view(..),
        },
    );
    let radiance_cascades = RadianceCascades::new(
        cascades,
        &TraceWorld {
            size: grid_size,
            radiance: radiance.view(0),
            opacity: world.display_opacity.view(0),
            difference: difference.view(0),
            environment: world.environment.view(..),
        },
    );

    let update_bounce_environment_kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0..env_facing_ratio {
            *total_radiance += world
                .environment
                .read(dispatch_id().x * env_facing_ratio + i);
        }
        bounce_environment.write(dispatch_id().x, total_radiance / env_facing_ratio as f32);
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
        let avg_radiance = total_radiance / bounce_cascades.facing_count(level).cast_f32();

        let emissive = world.emissive.read(dispatch_id().xy());
        let diffuse = world.diffuse.read(dispatch_id().xy());

        radiance.write(dispatch_id().xy(), avg_radiance * diffuse + emissive);
    }));
    let finish_radiance_kernel = DEVICE.create_kernel::<fn(u32, bool)>(&track!(|level, raw| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / cascades.probe_spacing(level).cast_u32(),
                facing: i,
                level,
            });
            *total_radiance += radiance_cascades.radiance.read(ray);
        }
        let avg_radiance = total_radiance / cascades.facing_count(level).cast_f32();
        radiance.write(
            dispatch_id().xy(),
            if raw {
                avg_radiance
            } else {
                let emissive = world.display_emissive.read(dispatch_id().xy());
                let diffuse = world.display_diffuse.read(dispatch_id().xy());
                avg_radiance * diffuse + emissive
            },
        );
    }));

    let update_diff_kernel = DEVICE.create_kernel::<fn(Tex2d<Opacity>)>(&track!(|opacity| {
        let pos = dispatch_id().xy();
        let diff = false.var();
        let this_radiance = radiance.read(pos);
        let this_depth = opacity.read(pos);
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
                let neighbor_radiance = radiance.read(neighbor.cast_u32());
                let neighbor_opacity = opacity.read(neighbor.cast_u32());
                if (neighbor_radiance != this_radiance).any()
                    || (neighbor_opacity != this_depth).any()
                {
                    *diff = true;
                    break;
                }
            }
        }
        difference.write(pos, **diff);
    }));

    let mut display_level = 0;

    let display_kernel = DEVICE.create_kernel::<fn(bool)>(&track!(|show_diff| {
        app.display().write(
            dispatch_id().xy(),
            radiance.read(dispatch_id().xy())
                + if show_diff {
                    difference.read(dispatch_id().xy()).cast_u32().cast_f32() * 5.0
                } else {
                    0.0_f32.expr()
                },
        );
    }));

    let mut merge_variant = 0;
    let mut num_bounces = 0;
    let mut run_final = true;
    let mut show_diff = false;
    let mut raw_radiance = false;

    let mut t = 0;

    let mut total_runtime = vec![0.0];

    let draw_kernel = DEVICE.create_kernel::<fn(Vec2<f32>, f32, Vec3<f32>, Vec3<f32>, Vec3<f32>)>(
        &track!(|pos, radius, emiss, diff, opacity| {
            if (dispatch_id().xy().cast_f32() - pos).length() < radius {
                world.emissive.write(dispatch_id().xy(), emiss);
                world.diffuse.write(dispatch_id().xy(), diff);
                world.opacity.write(dispatch_id().xy(), opacity);
                world.display_opacity.write(dispatch_id().xy(), opacity);
            }
        }),
    );

    #[rustfmt::skip]
    let material_map = vec![
        (MouseButton::Left, (Vec3::splat(0.0), Vec3::splat(1.0), Vec3::splat(1000.0))),
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
        if rt.pressed_key(KeyCode::KeyX) {
            rt.begin_recording(None, false);
        }

        t += 1;

        // let pos = Vec2::new(200.0 + 100.0 * (t as f32 / 40.0).cos(), 200.0);

        // for button in material_map.keys() {
        //     if rt.pressed_button(*button) {
        //         let pos = rt.cursor_position;
        //         draw(pos, 10.0, *button);
        //     }
        // }

        if rt.just_pressed_key(KeyCode::Enter) {
            merge_variant = (merge_variant + 1) % radiance_cascades.merge_kernel_count();
        }
        if rt.just_pressed_key(KeyCode::KeyL) {
            display_level = (display_level + 1) % cascades.num_cascades;
            println!("Display level: {}", display_level);
        }
        if rt.just_pressed_key(KeyCode::KeyB) {
            num_bounces = (num_bounces + 1) % 4;
            total_runtime = vec![0.0; num_bounces + 1];
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
        if rt.just_pressed_key(KeyCode::KeyR) {
            raw_radiance = !raw_radiance;
            println!("Raw radiance: {}", raw_radiance);
        }

        let timings = (
            world
                .emissive
                .view(0)
                .copy_to_texture_async(&radiance.view(0)),
            update_bounce_environment_kernel
                .dispatch_async(grid_dispatch)
                .debug("Update environment map"),
            (0..num_bounces)
                .map(|_i| {
                    (
                        update_diff_kernel
                            .dispatch_async(grid_dispatch, &world.opacity)
                            .debug("Update diff"),
                        bounce_radiance_cascades.update(0),
                        update_radiance_kernel
                            .dispatch_async(grid_dispatch, &0)
                            .debug("Update radiance"),
                    )
                        .chain()
                })
                .collect::<Vec<_>>(),
            run_final.then(|| {
                (
                    update_diff_kernel
                        .dispatch_async(grid_dispatch, &world.display_opacity)
                        .debug("Update diff"),
                    radiance_cascades.update(merge_variant),
                    finish_radiance_kernel
                        .dispatch_async(grid_dispatch, &display_level, &raw_radiance)
                        .debug("Finish radiance"),
                )
                    .chain()
            }),
        )
            .chain()
            .execute_timed();
        if rt.just_pressed_key(KeyCode::Space) {
            println!("{:?}", timings);
        }
        {
            let mut index = 0;
            let mut last_merge = false;
            for (name, value) in timings.iter() {
                if name.starts_with("merge") {
                    total_runtime[index] += *value;
                    last_merge = true;
                } else {
                    if last_merge {
                        index += 1;
                    }
                    last_merge = false;
                }
            }
        }
        if t % 100 == 0 {
            println!("Runtime:");
            if num_bounces > 0 {
                for (i, time) in total_runtime.iter().enumerate().take(num_bounces) {
                    println!("  Bounce {}: {}ms", i, time / 100.0);
                }
            }
            println!("  Display: {}ms", total_runtime[num_bounces] / 100.0);
            println!("  Total: {}ms", total_runtime.iter().sum::<f32>() / 100.0);
            total_runtime.fill(0.0);
        }

        scope.submit([display_kernel.dispatch_async(grid_dispatch, &show_diff)]);
    });
}
