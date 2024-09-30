#![feature(array_chunks)]

use std::{
    collections::HashMap,
    f32::consts::TAU,
    fs::File,
    path::{Path, PathBuf},
};

use cascade::{CascadeSettings, CascadeSize, RayLocation, RayLocationComps};
use color::{Diffuse, Opacity, Radiance};
use data::{Material, Settings};
use glam::Vec3 as FVec3;
use luisa::lang::types::vector::{Vec2, Vec3};
use radiance::RadianceCascades;
use sefirot::prelude::*;
use sefirot_testbed::{App, KeyCode, MouseButton};
use serde::{Deserialize, Serialize};
use tiff::{
    decoder::{Decoder as TiffDecoder, DecodingResult},
    encoder::{colortype, TiffEncoder},
    tags::Tag,
    ColorType,
};
use trace::{Block, BlockType, TraceWorld};
use world::World;

mod cascade;
mod color;
mod data;
mod radiance;
mod trace;
mod utils;
mod world;

pub fn load_env(path: impl AsRef<Path> + Copy) -> Vec<FVec3> {
    let file = File::open(path.as_ref().with_extension("tiff")).unwrap();
    let mut file = TiffDecoder::new(file).unwrap();
    assert_eq!(file.colortype().unwrap(), ColorType::RGB(32));
    let image = file.read_image().unwrap();
    let DecodingResult::F32(image) = image else {
        unreachable!()
    };
    image
        .array_chunks::<3>()
        .copied()
        .map(FVec3::from)
        .collect::<Vec<_>>()
}
pub fn downsample_env(data: &[FVec3], buffer: &Buffer<Radiance>) {
    let ratio = data.len() / buffer.len();
    assert_eq!(data.len() % buffer.len(), 0);
    let staging = (0..buffer.len())
        .map(|i| {
            Vec3::from(
                data[i * ratio..(i + 1) * ratio]
                    .iter()
                    .copied()
                    .sum::<FVec3>()
                    / ratio as f32,
            )
        })
        .collect::<Vec<_>>();
    buffer.copy_from(&staging);
}
pub fn save_env(env: &[FVec3], path: impl AsRef<Path> + Copy) {
    let data = env
        .iter()
        .copied()
        .flat_map(<[f32; 3]>::from)
        .collect::<Vec<_>>();
    let width = 1 << (env.len().trailing_zeros() / 2);
    let file = File::create(path.as_ref()).unwrap();
    let mut file = TiffEncoder::new(file).unwrap();
    file.write_image::<colortype::RGB32Float>(width, env.len() as u32 / width, &data)
        .unwrap();
}

pub fn main() {
    let env_file_name = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "env/default.tiff".to_string());
    let world_file_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "world/default.tiff".to_string());
    let settings_file_name = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "settings/default.ron".to_string());

    let settings: Settings = File::open(settings_file_name)
        .ok()
        .map(ron::de::from_reader)
        .map(Result::unwrap)
        .unwrap_or_default();

    let grid_size = settings.world_size;
    let grid_dispatch = [grid_size[0], grid_size[1], 1];

    let cascades = settings.cascades;
    let bounce_cascades = settings.bounce_cascades;

    let app = App::new("Amida", grid_size)
        .scale(settings.pixel_size)
        .dpi_override(settings.dpi)
        .agx()
        .init();

    let world = World::new(grid_size[0], grid_size[1]);
    let environment =
        DEVICE.create_buffer(cascades.level_size(cascades.num_cascades).facings as usize);
    let bounce_environment = DEVICE.create_buffer(
        bounce_cascades
            .level_size(bounce_cascades.num_cascades)
            .facings as usize,
    );

    if std::fs::exists(&env_file_name).unwrap_or(false) {
        let data = load_env(&env_file_name);
        downsample_env(&data, &environment);
        downsample_env(&data, &bounce_environment);
    }
    if std::fs::exists(&world_file_name).unwrap_or(false) {
        world.load(&world_file_name);
    }

    let radiance =
        DEVICE.create_tex2d::<Radiance>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);
    let difference = DEVICE.create_tex2d::<<BlockType as Block>::Storage>(
        BlockType::STORAGE_FORMAT,
        grid_size[0] / BlockType::SIZE,
        grid_size[1] / BlockType::SIZE,
        1,
    );
    let difference_nonempty = DEVICE.create_tex2d::<bool>(
        PixelStorage::Byte1,
        grid_size[0] / BlockType::SIZE,
        grid_size[1] / BlockType::SIZE,
        1,
    );

    let bounce_radiance_cascades = RadianceCascades::new(
        bounce_cascades,
        &TraceWorld {
            size: grid_size,
            radiance: radiance.view(0),
            opacity: world.opacity.view(0),
            environment: bounce_environment.view(..),
            difference: difference.view(0),
            nonempty_blocks: difference_nonempty.view(0),
        },
    );
    let radiance_cascades = RadianceCascades::new(
        cascades,
        &TraceWorld {
            size: grid_size,
            radiance: radiance.view(0),
            opacity: world.display_opacity.view(0),
            environment: environment.view(..),
            difference: difference.view(0),
            nonempty_blocks: difference_nonempty.view(0),
        },
    );

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
        let block = BlockType::empty().var();
        for dx in 0..BlockType::SIZE {
            for dy in 0..BlockType::SIZE {
                let pos = dispatch_id().xy() * BlockType::SIZE + Vec2::expr(dx, dy);
                let diff = false.var();
                let this_radiance = radiance.read(pos);
                let this_opacity = opacity.read(pos);
                for i in 0_u32..4_u32 {
                    let offset = [
                        Vec2::new(1, 0),
                        Vec2::new(-1, 0),
                        Vec2::new(0, 1),
                        Vec2::new(0, -1),
                    ]
                    .expr()[i];
                    let neighbor = pos.cast_i32() + offset;
                    if (neighbor >= 0).all()
                        && (neighbor < Vec2::from(grid_size).expr().cast_i32()).all()
                    {
                        let neighbor_radiance = radiance.read(neighbor.cast_u32());
                        let neighbor_opacity = opacity.read(neighbor.cast_u32());
                        if (neighbor_radiance != this_radiance).any()
                            || (neighbor_opacity != this_opacity).any()
                        {
                            *diff = true;
                            break;
                        }
                    }
                }
                if diff {
                    BlockType::set(block, Vec2::expr(dx, dy));
                }
            }
        }
        difference_nonempty.write(dispatch_id().xy(), !BlockType::is_empty(**block));
        BlockType::write(&difference.view(0), dispatch_id().xy(), **block);
    }));

    let display_kernel = DEVICE.create_kernel::<fn(bool)>(&track!(|show_diff| {
        app.display().write(
            dispatch_id().xy(),
            radiance.read(dispatch_id().xy())
                + if show_diff {
                    let block =
                        BlockType::read(&difference.view(0), dispatch_id().xy() / BlockType::SIZE);
                    BlockType::get(block, dispatch_id().xy() % BlockType::SIZE)
                        .cast_u32()
                        .cast_f32()
                        * 5.0
                        + (!BlockType::is_empty(block)).cast_u32().cast_f32() * 1.0
                } else {
                    0.0_f32.expr()
                },
        );
    }));

    let mut merge_variant = settings.merge_variant;
    let mut num_bounces = settings.num_bounces;
    let mut run_final = settings.run_final;
    let mut show_diff = settings.show_diff;
    let mut raw_radiance = settings.raw_radiance;
    let mut display_level = settings.display_level;
    let mut brush_radius = settings.brush_radius;

    let mut t = 0;

    #[cfg(not(feature = "trace"))]
    let mut total_runtime = 0.0;
    #[cfg(feature = "trace")]
    let mut total_runtime = vec![0.0; num_bounces + 1];

    #[rustfmt::skip]
    let draw_kernel = DEVICE.create_kernel::<fn(Vec2<f32>, f32, Vec3<f32>, Vec3<f32>, Vec3<f32>, Vec3<f32>, Vec3<f32>, Vec3<f32>)>(
        &track!(|pos, radius, emissive, diffuse, opacity, display_emissive, display_diffuse, display_opacity| {
            if (dispatch_id().xy().cast_f32() - pos).length() < radius {
                world.emissive.write(dispatch_id().xy(), emissive);
                world.diffuse.write(dispatch_id().xy(), diffuse);
                world.opacity.write(dispatch_id().xy(), opacity);
                world.display_emissive.write(dispatch_id().xy(), display_emissive);
                world.display_diffuse.write(dispatch_id().xy(), display_diffuse);
                world.display_opacity.write(dispatch_id().xy(), display_opacity);
            }
        }),
    );

    #[rustfmt::skip]
    let draw = |pos: Vec2<f32>, r: f32, m: &Material| {
        draw_kernel.dispatch(
            grid_dispatch,
            &pos,
            &r,
            &Vec3::from(m.emissive), &Vec3::from(m.diffuse), &Vec3::from(m.opacity),
            &Vec3::from(m.display_emissive), &Vec3::from(m.display_diffuse), &Vec3::from(m.display_opacity),
        );
    };

    app.run(|rt, scope| {
        drop(scope);

        t += 1;

        #[cfg(feature = "record")]
        if rt.pressed_key(KeyCode::KeyX) {
            println!("Recording");
            rt.begin_recording(None, false);
        }

        if rt.mouse_scroll != Vec2::splat(0.0) {
            brush_radius = (brush_radius + rt.mouse_scroll.y).max(1.0);
            println!("Brush radius: {}", brush_radius);
        }
        if rt.just_pressed_key(KeyCode::Enter) {
            merge_variant = (merge_variant + 1) % radiance_cascades.merge_kernel_count();
            println!("Merge variant: {}", merge_variant);
        } else if rt.just_pressed_key(KeyCode::KeyE) {
            display_level = (display_level + 1) % cascades.num_cascades;
            println!("Display level: {}", display_level);
        } else if rt.just_pressed_key(KeyCode::KeyB) {
            num_bounces = (num_bounces + 1) % 4;
            #[cfg(feature = "trace")]
            {
                total_runtime = vec![0.0; num_bounces + 1];
            }
            println!("Bounces: {}", num_bounces);
        } else if rt.just_pressed_key(KeyCode::KeyF) {
            run_final = !run_final;
            println!("Display final bounce: {}", run_final);
        } else if rt.just_pressed_key(KeyCode::KeyD) {
            show_diff = !show_diff;
            println!("Show difference map: {}", show_diff);
        } else if rt.just_pressed_key(KeyCode::KeyR) {
            raw_radiance = !raw_radiance;
            println!("Display raw radiance: {}", raw_radiance);
        } else if rt.just_pressed_key(KeyCode::KeyS) {
            let mut path = PathBuf::from(&world_file_name);
            if !rt.pressed_key(KeyCode::ControlLeft) {
                let ext = path.extension().unwrap_or_default();
                let mut file_name = path.file_stem().unwrap().to_owned();
                file_name.push("-");
                file_name.push(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("Temporal anomaly detected.")
                        .as_millis()
                        .to_string(),
                );
                file_name.push(".");
                file_name.push(ext);
                path.set_file_name(file_name);
            }
            world.save(&path);
            println!("Saved to {:?}", path);
        } else if rt.just_pressed_key(KeyCode::KeyL) {
            world.load(&world_file_name);
            println!("Loaded");
        } else {
            for (key, material) in &settings.key_materials {
                if rt.pressed_key(*key) {
                    let pos = rt.cursor_position;
                    draw(pos, brush_radius, material);
                }
            }
        }
        for (button, material) in &settings.mouse_materials {
            if rt.pressed_button(*button) {
                let pos = rt.cursor_position;
                draw(pos, brush_radius, material);
            }
        }

        let commands = (
            world
                .emissive
                .view(0)
                .copy_to_texture_async(&radiance.view(0)),
            (0..num_bounces)
                .map(|_i| {
                    (
                        update_diff_kernel
                            .dispatch_async(
                                [
                                    grid_size[0] / BlockType::SIZE,
                                    grid_size[1] / BlockType::SIZE,
                                    1,
                                ],
                                &world.opacity,
                            )
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
            display_kernel
                .dispatch_async(grid_dispatch, &show_diff)
                .debug("Display"),
        )
            .chain();
        #[cfg(not(feature = "trace"))]
        {
            let start = std::time::Instant::now();
            commands.execute();
            total_runtime += start.elapsed().as_secs_f32() * 1000.0;
            if t % 100 == 0 {
                println!("Frame time: {:?}ms", total_runtime / 100.0);
                total_runtime = 0.0;
            }
        }
        #[cfg(feature = "trace")]
        {
            let timings = commands.execute_timed();
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
        }
    });
}
